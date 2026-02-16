# RPG-Encoder Parity and Credibility Audit

**Date:** February 15, 2026  
**Scope:** Practical parity vs Microsoft RPG-ZeroRepo and paper-level RPG-Encoder claims.

## Executive Answer

No, benchmarking/credibility is **not** the only missing piece.

You are close on many core capabilities, but parity requires both:

1. **Capability parity** (the system can do the same class of work), and
2. **Evidence parity** (independent, reproducible results that prove it).

Today:

- **Understanding pipeline:** close to parity in architecture/mechanics.
- **Generation pipeline:** improved significantly, but still short of proven parity.
- **Credibility/evaluation:** the largest remaining gap.

## What Is Already Strong

### 1. Engineering and productization

- Broad MCP tool surface and operational workflow.
- Strong local test/CI hygiene.
- Multi-language parsing/graphing support.
- Incremental update and storage model maturity.

### 2. New capabilities now present

- Autonomous test-execute loop with failure routing and telemetry.
- Cost/latency/token efficiency reporting.
- Representation quality controls (ontology seeding, drift checks, confidence scoring).
- Retrieval/localization ablation framework.
- External validation bundle export for blinded reproduction.

These close major functional gaps and materially improve generation + evaluation readiness.

## Where Parity Is Still Not Complete

## A. Evidence gap (largest)

- No independent third-party reproduced results yet.
- No widely accepted benchmark publication proving equivalent outcomes.
- No blinded external report published from the new validation bundle flow.

**Impact:** Without this, claims of parity with paper-grade systems remain weak.

## B. Generation maturity gap

- Core loop exists, but comparative evidence at benchmark scale is not demonstrated.
- Ontology/feature-quality strategy is lighter than a large curated capability ontology.
- Runtime hardening exists, but large-scale stress data is not yet published.

**Impact:** System can run the workflow, but “same level” is not proven.

## C. Methodology risk in current ablations

- Current query construction uses deterministic first-feature sampling per entity.
- This is stable but can bias reported retrieval metrics.

**Impact:** Reported ablation numbers may not fully represent true robustness.

## D. Hardcode/TOML status (clarified)

- Docker runtime image selection is now TOML-configurable (`[generation.docker_images]`) and case-normalized.
- No default hardcoded runtime image fallback in the autonomous loop path.
- Language support itself is still bounded by compiled parser definitions (not fully dynamic from TOML).

**Impact:** Runtime mapping is configurable as intended; language universe is still implementation-defined.

## Capability Parity Matrix (Brutal)

| Area | Current Status | Parity Verdict |
|---|---|---|
| Graph build + hierarchy + lifting | Implemented and functional | Near parity |
| Incremental graph evolution | Implemented and tested | Near parity |
| Multi-language coverage | Strong | Exceeds Python-only systems |
| Autonomous generation loop | Implemented | Partial parity |
| Failure telemetry + cost reporting | Implemented | Partial parity (needs published runs) |
| Representation quality controls | Implemented | Partial parity (needs stronger eval protocol) |
| Retrieval/localization ablations | Implemented | Partial parity (sampling method should be upgraded) |
| External blinded validation flow | Implemented tooling | Not parity until executed and published |
| Independent reproduction | Not completed | Not parity |
| Public benchmark-grade proof | Not completed | Not parity |

## Direct Answer to “Can We Do the Same Thing?”

**Short answer:** Broadly yes in many workflow categories, but not yet at the same proven level.

- You can perform the same *class* of operations across understanding/generation loops.
- You cannot yet claim equivalent *outcome quality* without external benchmark proof.

## What Must Be True Before Claiming Parity

Minimum acceptance bar:

1. **Head-to-head benchmark runs** on fixed commit SHAs, with reproducible scripts.
2. **Blinded third-party evaluation** completed and published.
3. **Cost/quality/latency curves** across multiple backbones on the same task set.
4. **Ablation rigor upgrade** (randomized/stratified query selection, repeated trials).
5. **Replication package** (configs, artifacts, scoring scripts, environment manifest).

If these are done and results are competitive, parity claims become credible.

## What It Takes to Say We “Excel”

You can claim to excel only if you show one or more of:

1. Better quality at same cost.
2. Same quality at lower cost/latency.
3. Better cross-language generalization.
4. Better reliability under failure-mode stress tests.
5. Better reproducibility (independent labs reproduce your numbers).

Without that evidence, “excel” is a product opinion, not a defendable technical claim.

## 30-Day Hard Plan (Execution-Oriented)

1. Run blinded external validation on at least 2 independent evaluators.
2. Publish benchmark protocol and raw outputs for reproducibility.
3. Upgrade ablation sampling to randomized/stratified + confidence intervals.
4. Publish quality-vs-cost curves across at least 3 backbones.
5. Add a public “claim policy” in docs: what can/cannot be claimed today.

## Safe Public Position Today

You can safely claim:

- Strong engineering implementation of RPG-style understanding/generation workflows.
- Significant improvements in autonomous runtime, telemetry, and evaluation tooling.
- Multi-language and MCP operational advantages.

You should avoid claiming:

- Full paper-level parity.
- Superior benchmark performance.
- External reproducibility.

until the validation evidence is completed.

## Evidence Sources Reviewed

- `COMPARISON.md`
- `2509.16198.md`
- `2602.02084.md`
- https://github.com/microsoft/RPG-ZeroRepo
- Current repository changes and staged diff at the time of writing.

