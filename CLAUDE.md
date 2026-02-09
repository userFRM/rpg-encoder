# rpg-encoder

Rust workspace for building Repository Planning Graphs (RPGs) — semantic code understanding via MCP.

## Build & Verify

```bash
cargo fmt --all                                    # format
cargo clippy --workspace --all-targets -- -D warnings  # lint
cargo test --workspace                             # test (262 tests)
cargo build --release -p rpg-mcp                   # release binary
```

**Before every commit**: run fmt, clippy, and test. All three must pass. Do not skip or suppress warnings.

## Git Workflow

### Branch naming
- `fix/*` — bug fixes
- `feat/*` — new features
- `docs/*` — documentation only
- `chore/*` — tooling, deps, version bumps

### Commits
Use [conventional commits](https://www.conventionalcommits.org/):
- `fix:` bug fix
- `feat:` new feature
- `chore:` tooling, version bump, deps
- `docs:` documentation
- `refactor:` code restructure (no behavior change)
- `test:` adding or fixing tests

### Issue-first workflow
1. Open a GitHub issue describing the problem or feature
2. Create a branch from `main` (`fix/issue-description` or `feat/issue-description`)
3. Make commits on the branch
4. Open a PR referencing the issue (`Closes #N` or `Ref #N`)
5. Merge via squash merge (`gh pr merge --squash --delete-branch`)
6. Pull main locally after merge

### Release process
1. Bump version in root `Cargo.toml` (`[workspace.package] version`) AND `npm/package.json`
2. Run `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
3. Commit: `chore: bump version to X.Y.Z`
4. Tag: `git tag vX.Y.Z`
5. Push: `git push origin main && git push origin vX.Y.Z`
6. The release workflow auto-builds binaries, creates a GitHub release, and publishes to npm

## Project Structure

```
crates/
  rpg-core/      # Graph data model, storage, schema, LCA
  rpg-parser/    # Tree-sitter entity extraction (Rust, Python, JS/TS, Go, Java, C/C++)
  rpg-encoder/   # Semantic lifting, grounding, dependency resolution
  rpg-nav/       # Search, explore, fetch, export (navigation layer)
  rpg-cli/       # CLI binary
  rpg-mcp/       # MCP server binary (stdio JSON-RPC)
```

### Key files
- `crates/rpg-encoder/src/prompts/*.md` — LLM prompt templates (included via `include_str!`)
- `crates/rpg-mcp/src/prompts/server_instructions.md` — LIFTER PROTOCOL
- `crates/rpg-core/src/graph.rs` — Core graph data model (Entity, RPGraph, EdgeKind)
- `crates/rpg-encoder/src/grounding.rs` — Dependency resolution + artifact grounding

### Architecture notes
- All serialized maps use `BTreeMap` for deterministic JSON output
- Edges are sorted by `(source, target, kind)` before serialization
- `lifting_coverage()` excludes Module entities (they get features via aggregation)
- The connected coding agent IS the LLM for lifting (no API key needed)
- Edge kinds: Imports, Invokes, Inherits, Composes, Contains

## CI Checks

The GitHub Actions workflow runs: `fmt --check`, `clippy -D warnings`, `test --workspace`. All must pass for PRs.
