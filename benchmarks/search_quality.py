#!/usr/bin/env python3
"""
RPG-Encoder Search Quality Benchmark

Measures whether semantic lifting improves search localization accuracy.
Compares unlifted (snippet-only) vs lifted (semantic features) search.

Uses the rpg-encoder repository itself as the benchmark target.

Architecture:
    Phase 1 — PREPARE (slow, cached, run once):
        Build graphs, lift entities with LLM.
        Results are cached in /tmp/rpg-bench/<repo>/.rpg/

    Phase 2 — MEASURE (fast, reproducible, run many times):
        Run search queries against cached graphs, compute Acc@k.

Usage:
    # First run: prepare + measure (slow — builds, lifts)
    python3 benchmarks/search_quality.py

    # Re-run measurement only (fast — uses cached .rpg graphs)
    python3 benchmarks/search_quality.py --measure-only

    # Rebuild graphs but reuse lifted features
    python3 benchmarks/search_quality.py --force-rebuild

    # Re-lift all entities (rebuilds + re-lifts)
    python3 benchmarks/search_quality.py --force-lift

    # Skip lifting entirely (unlifted baseline only)
    python3 benchmarks/search_quality.py --no-lift

    # Custom binary
    python3 benchmarks/search_quality.py --rpg-binary ./target/debug/rpg-encoder

Requires:
    - rpg-encoder binary (cargo build --release)
    - An LLM provider (Moonshot, OpenAI, Anthropic, or Ollama) for lifting
"""

import argparse
import json
import os
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

BENCH_DIR = Path("/tmp/rpg-bench")
SCRIPT_DIR = Path(__file__).parent
QUERIES_FILE = SCRIPT_DIR / "queries.json"
DEFAULT_BINARY = str(SCRIPT_DIR.parent / "target" / "release" / "rpg-encoder")


# ── Helpers ──────────────────────────────────────────────────────────────────

def find_binary():
    """Find rpg-encoder binary."""
    for candidate in [
        DEFAULT_BINARY,
        DEFAULT_BINARY.replace("/release/", "/debug/"),
        shutil.which("rpg-encoder"),
    ]:
        if candidate and os.path.exists(candidate):
            return candidate
    print("ERROR: rpg-encoder binary not found. Run: cargo build --release")
    sys.exit(1)


def run_cmd(args, timeout=300, stream_stderr=False):
    """Run a command and return (stdout, stderr, returncode, elapsed)."""
    start = time.time()
    try:
        if stream_stderr:
            # Stream stderr line-by-line for progress
            proc = subprocess.Popen(
                args,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            stderr_lines = []
            while True:
                line = proc.stderr.readline()
                if not line and proc.poll() is not None:
                    break
                if line:
                    stderr_lines.append(line)
                    line = line.strip()
                    if line.startswith("Lifting batch"):
                        # Print progress on same line
                        print(f"\r    {line}", end="", flush=True)
                    elif line and not line.startswith("Using LLM"):
                        pass  # suppress other noise
            stdout = proc.stdout.read()
            stderr = "".join(stderr_lines)
            rc = proc.returncode
            elapsed = time.time() - start
            return stdout, stderr, rc, elapsed
        else:
            result = subprocess.run(
                args,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
            elapsed = time.time() - start
            return result.stdout, result.stderr, result.returncode, elapsed
    except subprocess.TimeoutExpired:
        return "", "TIMEOUT", 1, timeout


def parse_entity_count(stderr):
    """Extract entity count from build stderr."""
    for line in stderr.split("\n"):
        if "Entities:" in line and "Lifted:" not in line:
            m = re.search(r"Entities:\s*(\d+)", line)
            if m:
                return int(m.group(1))
    return 0


def parse_lifted_count(stderr):
    """Extract lifted count from lift stderr."""
    for line in stderr.split("\n"):
        if "Entities lifted:" in line:
            m = re.search(r"Entities lifted:\s*(\d+)", line)
            if m:
                return int(m.group(1))
    return 0


def parse_search_results(stdout):
    """Parse search output into structured results."""
    results = []
    for line in stdout.strip().split("\n"):
        if not line.strip():
            continue
        m = re.match(
            r"^\d+\.\s+(\S+)\s+\[(.+?):(\d+)\]\s+\(score:\s+([\d.]+)\)", line
        )
        if m:
            results.append({
                "name": m.group(1),
                "file": m.group(2),
                "line": int(m.group(3)),
                "score": float(m.group(4)),
            })
    return results


def find_rank(results, expected_files):
    """Find rank (1-indexed) of first matching result, or 0 if miss."""
    for i, r in enumerate(results):
        # Extract the filename from the path (e.g., "src/search.rs" -> "search.rs")
        result_filename = os.path.basename(r["file"])
        for exp in expected_files:
            if exp == result_filename:
                return i + 1
    return 0


def graph_exists(repo_dir):
    """Check if an RPG graph exists for this repo."""
    return (repo_dir / ".rpg" / "graph.json").exists()


def graph_is_lifted(repo_dir):
    """Check if the RPG graph has any lifted entities."""
    graph_file = repo_dir / ".rpg" / "graph.json"
    if not graph_file.exists():
        return False
    try:
        with open(graph_file) as f:
            g = json.load(f)
        for e in g.get("entities", {}).values():
            if isinstance(e, dict) and e.get("semantic_features"):
                return True
    except Exception:
        pass
    return False


def count_lifted(repo_dir):
    """Count lifted entities in graph."""
    graph_file = repo_dir / ".rpg" / "graph.json"
    if not graph_file.exists():
        return 0, 0
    try:
        with open(graph_file) as f:
            g = json.load(f)
        entities = g.get("entities", {})
        total = len(entities)
        lifted = sum(
            1 for e in entities.values()
            if isinstance(e, dict) and e.get("semantic_features")
        )
        return total, lifted
    except Exception:
        return 0, 0


# ── Phase 1: Prepare ────────────────────────────────────────────────────────

def get_repo_dir(repo_config):
    """Get the working directory for a repo (clone or copy as needed)."""
    name = repo_config["name"]

    # Local repo: copy to bench dir (preserves source, isolates .rpg data)
    if "local_path" in repo_config:
        local_path = Path(repo_config["local_path"]).resolve()
        if not local_path.exists():
            # Resolve relative to script dir's parent (project root)
            local_path = SCRIPT_DIR.parent / repo_config["local_path"]
            local_path = local_path.resolve()
        repo_dir = BENCH_DIR / name
        if repo_dir.exists():
            return repo_dir
        print(f"    Copying {local_path} -> {repo_dir}...")
        BENCH_DIR.mkdir(parents=True, exist_ok=True)
        # Copy source files only (skip .rpg, target, .git)
        subprocess.run(
            ["rsync", "-a", "--exclude", ".rpg", "--exclude", "target",
             "--exclude", ".git", str(local_path) + "/", str(repo_dir) + "/"],
            capture_output=True,
            timeout=120,
        )
        return repo_dir

    # Remote repo: clone
    short_name = name.split("/")[1] if "/" in name else name
    repo_dir = BENCH_DIR / short_name
    if repo_dir.exists():
        return repo_dir
    url = repo_config["url"]
    print(f"    Cloning {name}...")
    BENCH_DIR.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "clone", "--depth", "1", url, str(repo_dir)],
        capture_output=True,
        timeout=120,
    )
    return repo_dir


def build_graph(binary, repo_dir, language):
    """Build structural RPG graph. Excludes test/bench/example/fuzz files."""
    rpg_dir = repo_dir / ".rpg"
    if rpg_dir.exists():
        shutil.rmtree(rpg_dir)
    stdout, stderr, rc, elapsed = run_cmd(
        [binary, "build", "--lang", language, "-p", str(repo_dir),
         "--exclude", "*test*", "--exclude", "*bench*",
         "--exclude", "*example*", "--exclude", "*fuzz*"]
    )
    entities = parse_entity_count(stderr)
    return entities, elapsed, rc, stderr


def lift_all(binary, repo_dir):
    """Lift all entities with streaming progress."""
    stdout, stderr, rc, elapsed = run_cmd(
        [binary, "lift", "all", "-p", str(repo_dir)],
        timeout=3600,
        stream_stderr=True,
    )
    lifted = parse_lifted_count(stderr)
    return lifted, elapsed, rc


def prepare_repos(binary, config, force_rebuild=False, force_lift=False, no_lift=False):
    """Phase 1: Clone, build, lift all repos. Returns repo_dirs dict."""
    print("Phase 1: PREPARE")
    print("─" * 78)
    repo_dirs = {}

    for repo_config in config["repos"]:
        name = repo_config["name"]
        language = repo_config["language"]

        print(f"\n  [{name}] ({language})")

        # Get repo directory (clone or copy)
        repo_dir = get_repo_dir(repo_config)
        repo_dirs[name] = repo_dir

        # Build (skip if graph exists and not forced)
        needs_build = force_rebuild or force_lift or not graph_exists(repo_dir)
        if needs_build:
            print(f"    Building graph...", end=" ", flush=True)
            entities, build_time, rc, stderr = build_graph(binary, repo_dir, language)
            if rc != 0:
                print(f"FAILED (rc={rc})")
                print(f"    stderr: {stderr[:200]}")
                continue
            print(f"{entities} entities in {build_time:.1f}s")
        else:
            total, lifted = count_lifted(repo_dir)
            status = f", {lifted} lifted" if lifted > 0 else ""
            print(f"    Graph cached ({total} entities{status})")

        # Lift (skip if already lifted and not forced)
        if no_lift:
            print(f"    Lifting: SKIPPED (--no-lift)")
        elif force_lift or (not graph_is_lifted(repo_dir) and not no_lift):
            total, _ = count_lifted(repo_dir)
            print(f"    Lifting {total} entities with Ollama...", flush=True)
            lifted, lift_time, rc = lift_all(binary, repo_dir)
            if rc != 0:
                print(f"\n    Lifting FAILED (rc={rc})")
            else:
                print(f"\n    {lifted} entities lifted in {lift_time:.1f}s")
        else:
            _, lifted = count_lifted(repo_dir)
            print(f"    Already lifted ({lifted} entities)")

    print()
    return repo_dirs


# ── Phase 2: Measure ────────────────────────────────────────────────────────

def measure_search(binary, config, repo_dirs):
    """Phase 2: Run all search queries and compute metrics."""
    print("Phase 2: MEASURE")
    print("─" * 78)

    all_unlifted = {"@1": 0, "@3": 0, "@5": 0, "@10": 0, "total": 0, "mrr_sum": 0.0}
    all_lifted = {"@1": 0, "@3": 0, "@5": 0, "@10": 0, "total": 0, "mrr_sum": 0.0}
    repo_results = []

    for repo_config in config["repos"]:
        name = repo_config["name"]
        language = repo_config["language"]
        queries = repo_config["queries"]
        repo_dir = repo_dirs.get(name)

        if not repo_dir or not graph_exists(repo_dir):
            print(f"\n  [{name}] SKIP — no graph")
            continue

        total_entities, lifted_entities = count_lifted(repo_dir)
        has_lifted = lifted_entities > 0

        print(f"\n  [{name}] {total_entities} entities, {lifted_entities} lifted")

        # Run unlifted search (snippets mode — ignores semantic features)
        unlifted_results = []
        for q in queries:
            results = parse_search_results(
                run_cmd([binary, "search", q["query"], "--mode", "snippets",
                         "-p", str(repo_dir)])[0]
            )
            rank = find_rank(results, q["expect"])
            unlifted_results.append({
                "query": q["query"],
                "expect": q["expect"],
                "rank": rank,
                "top5": [r["file"] for r in results[:5]],
            })

        # Run lifted search (auto mode — uses features if available)
        lifted_results = []
        if has_lifted:
            for q in queries:
                results = parse_search_results(
                    run_cmd([binary, "search", q["query"], "--mode", "auto",
                             "-p", str(repo_dir)])[0]
                )
                rank = find_rank(results, q["expect"])
                lifted_results.append({
                    "query": q["query"],
                    "expect": q["expect"],
                    "rank": rank,
                    "top5": [r["file"] for r in results[:5]],
                })

        # Print per-query table
        print()
        if has_lifted:
            print(f"    {'Query':<40} {'Unlifted':>8} {'Lifted':>8} {'Delta':>7}  Expected")
            print(f"    {'─' * 40} {'─' * 8} {'─' * 8} {'─' * 7}  {'─' * 25}")
        else:
            print(f"    {'Query':<40} {'Unlifted':>8}  Expected")
            print(f"    {'─' * 40} {'─' * 8}  {'─' * 25}")

        for i, q in enumerate(queries):
            ur = unlifted_results[i]
            u_str = f"@{ur['rank']}" if ur["rank"] > 0 else "miss"

            if lifted_results:
                lr = lifted_results[i]
                l_str = f"@{lr['rank']}" if lr["rank"] > 0 else "miss"
                # Compute delta
                if ur["rank"] == 0 and lr["rank"] == 0:
                    d_str = ""
                elif ur["rank"] == 0 and lr["rank"] > 0:
                    d_str = "NEW"
                elif ur["rank"] > 0 and lr["rank"] == 0:
                    d_str = "LOST"
                elif lr["rank"] < ur["rank"]:
                    d_str = f"+{ur['rank'] - lr['rank']}"
                elif lr["rank"] > ur["rank"]:
                    d_str = f"-{lr['rank'] - ur['rank']}"
                else:
                    d_str = "="
                exp_str = ", ".join(q["expect"][:2])
                print(f"    {q['query']:<40} {u_str:>8} {l_str:>8} {d_str:>7}  {exp_str}")
            else:
                exp_str = ", ".join(q["expect"][:2])
                print(f"    {q['query']:<40} {u_str:>8}  {exp_str}")

        # Compute accuracy metrics
        total = len(queries)

        def compute_metrics(results):
            at1 = sum(1 for r in results if 0 < r["rank"] <= 1)
            at3 = sum(1 for r in results if 0 < r["rank"] <= 3)
            at5 = sum(1 for r in results if 0 < r["rank"] <= 5)
            at10 = sum(1 for r in results if 0 < r["rank"] <= 10)
            mrr = sum(1.0 / r["rank"] for r in results if r["rank"] > 0)
            return {"@1": at1, "@3": at3, "@5": at5, "@10": at10, "total": total, "mrr": mrr}

        u_metrics = compute_metrics(unlifted_results)
        l_metrics = compute_metrics(lifted_results) if lifted_results else None

        # Accumulate
        for k in ["@1", "@3", "@5", "@10"]:
            all_unlifted[k] += u_metrics[k]
        all_unlifted["total"] += total
        all_unlifted["mrr_sum"] += u_metrics["mrr"]

        if l_metrics:
            for k in ["@1", "@3", "@5", "@10"]:
                all_lifted[k] += l_metrics[k]
            all_lifted["total"] += total
            all_lifted["mrr_sum"] += l_metrics["mrr"]

        # Per-repo summary
        print()
        print(f"    {'Metric':<8} {'Unlifted':>12}", end="")
        if l_metrics:
            print(f" {'Lifted':>12} {'Delta':>8}", end="")
        print()
        print(f"    {'─' * 8} {'─' * 12}", end="")
        if l_metrics:
            print(f" {'─' * 12} {'─' * 8}", end="")
        print()

        for k in ["@1", "@3", "@5", "@10"]:
            u_pct = u_metrics[k] / total * 100
            u_s = f"{u_metrics[k]}/{total} ({u_pct:.0f}%)"
            print(f"    Acc{k:<5} {u_s:>12}", end="")
            if l_metrics:
                l_pct = l_metrics[k] / total * 100
                delta = l_pct - u_pct
                l_s = f"{l_metrics[k]}/{total} ({l_pct:.0f}%)"
                d_s = f"{delta:+.0f}%"
                print(f" {l_s:>12} {d_s:>8}", end="")
            print()

        u_mrr = u_metrics["mrr"] / total
        print(f"    {'MRR':<8} {u_mrr:>12.3f}", end="")
        if l_metrics:
            l_mrr = l_metrics["mrr"] / total
            print(f" {l_mrr:>12.3f} {l_mrr - u_mrr:>+8.3f}", end="")
        print()

        repo_results.append({
            "name": name,
            "language": language,
            "entities": total_entities,
            "lifted_count": lifted_entities,
            "queries": total,
            "unlifted": u_metrics,
            "lifted": l_metrics,
            "per_query": {
                "unlifted": [{"query": r["query"], "rank": r["rank"]} for r in unlifted_results],
                "lifted": [{"query": r["query"], "rank": r["rank"]} for r in lifted_results] if lifted_results else [],
            },
        })

    return all_unlifted, all_lifted, repo_results


def bootstrap_mrr_ci(unlifted_ranks, lifted_ranks, n_iterations=1000, ci=0.95):
    """Compute bootstrap confidence interval for MRR difference (lifted - unlifted).

    Returns (delta_mean, ci_lower, ci_upper).
    """
    if not unlifted_ranks or not lifted_ranks:
        return 0.0, 0.0, 0.0

    n = len(unlifted_ranks)
    assert n == len(lifted_ranks), "rank lists must be same length"

    def mrr(ranks):
        return sum(1.0 / r if r > 0 else 0.0 for r in ranks) / len(ranks)

    observed_delta = mrr(lifted_ranks) - mrr(unlifted_ranks)

    random.seed(42)  # reproducible
    deltas = []
    for _ in range(n_iterations):
        indices = [random.randint(0, n - 1) for _ in range(n)]
        u_sample = [unlifted_ranks[i] for i in indices]
        l_sample = [lifted_ranks[i] for i in indices]
        deltas.append(mrr(l_sample) - mrr(u_sample))

    deltas.sort()
    alpha = 1 - ci
    lower_idx = int(alpha / 2 * n_iterations)
    upper_idx = int((1 - alpha / 2) * n_iterations)
    return observed_delta, deltas[lower_idx], deltas[upper_idx]


def print_summary(all_unlifted, all_lifted, repo_results):
    """Print final summary."""
    print()
    print("=" * 78)
    print("SUMMARY")
    print("=" * 78)

    total_u = all_unlifted["total"]
    total_l = all_lifted["total"]

    print()
    print(f"  {'Metric':<8} {'Unlifted':>14}", end="")
    if total_l > 0:
        print(f" {'Lifted':>14} {'Delta':>8}", end="")
    print()
    print(f"  {'─' * 8} {'─' * 14}", end="")
    if total_l > 0:
        print(f" {'─' * 14} {'─' * 8}", end="")
    print()

    for k in ["@1", "@3", "@5", "@10"]:
        u_pct = all_unlifted[k] / total_u * 100 if total_u > 0 else 0
        u_s = f"{all_unlifted[k]}/{total_u} ({u_pct:.0f}%)"
        print(f"  Acc{k:<5} {u_s:>14}", end="")
        if total_l > 0:
            l_pct = all_lifted[k] / total_l * 100
            delta = l_pct - u_pct
            l_s = f"{all_lifted[k]}/{total_l} ({l_pct:.0f}%)"
            d_s = f"{delta:+.0f}%"
            print(f" {l_s:>14} {d_s:>8}", end="")
        print()

    u_mrr = all_unlifted["mrr_sum"] / total_u if total_u > 0 else 0
    print(f"  {'MRR':<8} {u_mrr:>14.3f}", end="")
    if total_l > 0:
        l_mrr = all_lifted["mrr_sum"] / total_l
        print(f" {l_mrr:>14.3f} {l_mrr - u_mrr:>+8.3f}", end="")
    print()

    # Bootstrap confidence interval for MRR delta
    if total_l > 0 and repo_results:
        all_u_ranks = []
        all_l_ranks = []
        for r in repo_results:
            pq = r.get("per_query", {})
            for ur in pq.get("unlifted", []):
                all_u_ranks.append(ur["rank"])
            for lr in pq.get("lifted", []):
                all_l_ranks.append(lr["rank"])

        if all_u_ranks and all_l_ranks and len(all_u_ranks) == len(all_l_ranks):
            delta, ci_lo, ci_hi = bootstrap_mrr_ci(all_u_ranks, all_l_ranks)
            print(f"\n  MRR delta: {delta:+.3f} (95% CI [{ci_lo:+.3f}, {ci_hi:+.3f}])")

    # Per-query delta summary (notable changes)
    if total_l > 0 and repo_results:
        improvements = []
        regressions = []
        for r in repo_results:
            pq = r.get("per_query", {})
            u_list = pq.get("unlifted", [])
            l_list = pq.get("lifted", [])
            for u, l in zip(u_list, l_list):
                u_rank = u["rank"]
                l_rank = l["rank"]
                if l_rank > 0 and (u_rank == 0 or l_rank < u_rank):
                    u_s = f"@{u_rank}" if u_rank > 0 else "miss"
                    improvements.append((u["query"], u_s, f"@{l_rank}"))
                elif u_rank > 0 and (l_rank == 0 or l_rank > u_rank):
                    l_s = f"@{l_rank}" if l_rank > 0 else "miss"
                    regressions.append((u["query"], f"@{u_rank}", l_s))

        if improvements:
            print(f"\n  Notable improvements ({len(improvements)}):")
            for q, u_s, l_s in sorted(improvements, key=lambda x: x[1]):
                print(f"    {q:<45} {u_s:>6} -> {l_s}")
        if regressions:
            print(f"\n  Regressions ({len(regressions)}):")
            for q, u_s, l_s in sorted(regressions, key=lambda x: x[1]):
                print(f"    {q:<45} {u_s:>6} -> {l_s}")

    print()


def save_results(all_unlifted, all_lifted, repo_results, binary):
    """Save machine-readable results."""
    results_file = SCRIPT_DIR / "results.json"
    total_u = all_unlifted["total"]
    total_l = all_lifted["total"]

    u_mrr = all_unlifted["mrr_sum"] / total_u if total_u > 0 else 0
    l_mrr = all_lifted["mrr_sum"] / total_l if total_l > 0 else 0

    # Compute bootstrap CI
    ci_data = None
    if total_l > 0 and repo_results:
        all_u_ranks = []
        all_l_ranks = []
        for r in repo_results:
            pq = r.get("per_query", {})
            for ur in pq.get("unlifted", []):
                all_u_ranks.append(ur["rank"])
            for lr in pq.get("lifted", []):
                all_l_ranks.append(lr["rank"])
        if all_u_ranks and all_l_ranks and len(all_u_ranks) == len(all_l_ranks):
            delta, ci_lo, ci_hi = bootstrap_mrr_ci(all_u_ranks, all_l_ranks)
            ci_data = {"delta": round(delta, 4), "ci_lower": round(ci_lo, 4), "ci_upper": round(ci_hi, 4)}

    data = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "binary": binary,
        "summary": {
            "unlifted": {k: all_unlifted[k] for k in ["@1", "@3", "@5", "@10", "total"]},
            "lifted": {k: all_lifted[k] for k in ["@1", "@3", "@5", "@10", "total"]},
            "unlifted_mrr": round(u_mrr, 4),
            "lifted_mrr": round(l_mrr, 4),
        },
        "repos": repo_results,
    }
    if ci_data:
        data["summary"]["mrr_bootstrap_ci"] = ci_data

    with open(results_file, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Results saved to {results_file}")
    return results_file


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="RPG-Encoder Search Quality Benchmark",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python3 benchmarks/search_quality.py              # Full run (prepare + measure)
  python3 benchmarks/search_quality.py --measure-only  # Fast re-run (search only)
  python3 benchmarks/search_quality.py --no-lift     # Unlifted baseline only
  python3 benchmarks/search_quality.py --force-lift  # Re-lift all entities
        """,
    )
    parser.add_argument("--rpg-binary", default=DEFAULT_BINARY,
                        help="Path to rpg-encoder binary")
    parser.add_argument("--measure-only", action="store_true",
                        help="Skip prepare phase, use cached graphs")
    parser.add_argument("--force-rebuild", action="store_true",
                        help="Force rebuild graphs (keeps lifted features)")
    parser.add_argument("--force-lift", action="store_true",
                        help="Force re-lift all entities")
    parser.add_argument("--no-lift", action="store_true",
                        help="Skip lifting entirely (unlifted baseline only)")
    parser.add_argument("--ci", action="store_true",
                        help="CI mode: exit with code 1 if lifting regresses MRR")
    args = parser.parse_args()

    binary = args.rpg_binary
    if not os.path.exists(binary):
        binary = find_binary()

    with open(QUERIES_FILE) as f:
        config = json.load(f)

    total_queries = sum(len(r["queries"]) for r in config["repos"])

    print("=" * 78)
    print("RPG-Encoder Search Quality Benchmark")
    print("=" * 78)
    print(f"  Binary:  {binary}")
    print(f"  Repos:   {len(config['repos'])}")
    print(f"  Queries: {total_queries}")
    if args.no_lift:
        print(f"  Lifting: DISABLED")
    elif args.measure_only:
        print(f"  Lifting: using cached")
    else:
        print(f"  Lifting: via Ollama (auto-detected model)")
    print()

    # Phase 1: Prepare
    if args.measure_only:
        # Build repo_dirs from existing copies/clones
        repo_dirs = {}
        for rc in config["repos"]:
            name = rc["name"]
            short_name = name.split("/")[1] if "/" in name else name
            repo_dir = BENCH_DIR / short_name
            if repo_dir.exists():
                repo_dirs[name] = repo_dir
            else:
                print(f"  WARNING: {repo_dir} not found — run without --measure-only first")
    else:
        repo_dirs = prepare_repos(
            binary, config,
            force_rebuild=args.force_rebuild,
            force_lift=args.force_lift,
            no_lift=args.no_lift,
        )

    # Phase 2: Measure
    all_unlifted, all_lifted, repo_results = measure_search(binary, config, repo_dirs)

    # Summary + save
    print_summary(all_unlifted, all_lifted, repo_results)
    save_results(all_unlifted, all_lifted, repo_results, binary)

    # CI mode: fail if lifting regresses MRR
    if args.ci and all_lifted["total"] > 0:
        u_mrr = all_unlifted["mrr_sum"] / all_unlifted["total"] if all_unlifted["total"] > 0 else 0
        l_mrr = all_lifted["mrr_sum"] / all_lifted["total"]
        if l_mrr < u_mrr:
            print(f"CI FAIL: lifted MRR ({l_mrr:.3f}) < unlifted MRR ({u_mrr:.3f})")
            sys.exit(1)
        else:
            print(f"CI PASS: lifted MRR ({l_mrr:.3f}) >= unlifted MRR ({u_mrr:.3f})")


if __name__ == "__main__":
    main()
