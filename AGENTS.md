# AGENTS.md

Repository guidance for Codex agents working on `vrm-rs`.

## Rust Work

- Before Rust implementation, testing, refactoring, or review work, read and follow the local `rust-best-practices` skill:
  - `C:\Users\sanze\.agents\skills\rust\SKILL.md`
- Keep changes idiomatic Rust: no `unsafe`, no unstable features, no broad macro-heavy abstractions unless explicitly justified.
- Run the normal gate before implementation commits through the local Rust CI script:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
```

- `just` is available as a convenience wrapper. Prefer `just ci` for the normal gate, while keeping the Rust script as the implementation source of truth:

```powershell
just ci
just ci-external
just render-parity
just render-parity-samples
```

- The repository intentionally does not carry GitHub Actions workflows. `tools/ci/local-ci.rs` fails fast if `.github/workflows/*.yml` or `.github/workflows/*.yaml` files are present. Use the local Rust CI script when you want the old CI-equivalent gate:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity
```

The default script run covers `cargo fmt --all -- --check`, `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, non-rendering example smokes (`mtoon_renderer_skeletons`, `wgpu_mtoon_pipeline_materialization`, `ash_mtoon_pipeline_materialization`, `bevy_mtoon_materialization`, `custom_engine_adapter`), capture-example compile/unit tests (`wgpu_render_capture`, `bevy_render_capture`), render-tool syntax/self-tests, and `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70`.

## Coverage Refresh Delegation

- Treat coverage table/progress updates as routine mechanical work suitable for delegation.
- Prefer the local `agent` CLI when available for routine coverage refreshes and other simple bulk mechanical edits, using `composer-2.5-fast` for clearly specified non-judgmental edits.
- Example:

```powershell
agent --print --trust --force --model composer-2.5-fast --workspace "D:\git\vrm-rs" "..."
```

- The delegated agent must follow `docs/agents/coverage-spark.md` and use `tools/coverage/update-coverage-docs.ps1`.
- Use Codex subagents only when the user explicitly requests them or when the `agent` CLI is unavailable.
- Preferred flow:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70 --json --output-path target/coverage-summary.json
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "YYYY-MM-DD"
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "YYYY-MM-DD" -Apply
git diff docs/testing.md docs/progress.md
```

- The first script run should be a dry run unless the user explicitly asked for direct application or the main agent has already reviewed the generated block.
- Keep `docs/testing.md` coverage snapshots and the newest relevant `docs/progress.md` coverage line synchronized.

## Main Agent Responsibility

- The main agent still owns the implementation gate and final verification.
- Review delegated diffs and run gates before staging or committing; delegation does not transfer ownership of verification.
- Run `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70` locally before commits that affect tests, coverage, runtime paths, adapters, IO, or protocol behavior.
- If a delegated agent updates coverage docs, review its diff before staging.
- If the `agent` CLI is unavailable and Codex subagent spawning is unavailable because the thread is at its agent limit, reuse an existing completed subagent when possible. If reuse is unavailable, perform the coverage refresh locally and record that limitation in the progress update.

## Subagent Usage

- Prefer the local `agent` CLI for routine mechanical work (coverage refreshes, bulk edits) when available.
- Use Codex subagents only when the user explicitly requests them, for clearly parallel routine tasks requested by the user, or when the `agent` CLI is unavailable.
- For coverage refreshes via Codex subagent, use a `worker` subagent with `gpt-5.3-codex-spark` when available.
- For pessimistic reviews, use the model and reasoning level requested by the user, and ask for findings focused on regressions, missing tests, and API hazards.
- Delegated agents should not make broad unrelated edits. Give them narrow ownership, ask them to report changed files, and review their output before integration.

## Fixtures And Licensing

- Keep official or third-party `.vrm`, `.vrma`, `.glb`, textures, and generated golden files under `.external-fixtures/`.
- Do not commit binary sample assets unless redistribution has been explicitly reviewed for this repository's MIT/Apache source distribution.
- Commit scripts, generated minimal JSON fixtures, and documentation only when they are source-like and license-safe.
