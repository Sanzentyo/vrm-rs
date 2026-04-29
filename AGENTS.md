# AGENTS.md

Repository guidance for Codex agents working on `vrm-rs`.

## Rust Work

- Before Rust implementation, testing, refactoring, or review work, read and follow the local `rust-best-practices` skill:
  - `C:\Users\sanze\.agents\skills\rust\SKILL.md`
- Keep changes idiomatic Rust: no `unsafe`, no unstable features, no broad macro-heavy abstractions unless explicitly justified.
- Run the normal gate before implementation commits:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Coverage Refresh Delegation

- Treat coverage table/progress updates as a routine subagent task.
- Prefer delegating coverage refreshes to a Codex subagent using model `gpt-5.3-codex-spark`.
- The subagent must follow `docs/agents/coverage-spark.md` and use `tools/coverage/update-coverage-docs.ps1`.
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
- Run `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70` locally before commits that affect tests, coverage, runtime paths, adapters, IO, or protocol behavior.
- If a subagent updates coverage docs, review its diff before staging.
- If subagent spawning is unavailable because the Codex thread is at its agent limit, reuse an existing completed subagent when possible. If reuse is unavailable, perform the coverage refresh locally and record that limitation in the progress update.

## Subagent Usage

- Use subagents only for explicit delegation requests or clearly parallel routine tasks requested by the user.
- For coverage refreshes, use a `worker` subagent with `gpt-5.3-codex-spark` when available.
- For pessimistic reviews, use the model and reasoning level requested by the user, and ask for findings focused on regressions, missing tests, and API hazards.
- Subagents should not make broad unrelated edits. Give them narrow ownership, ask them to report changed files, and review their output before integration.

## Fixtures And Licensing

- Keep official or third-party `.vrm`, `.vrma`, `.glb`, textures, and generated golden files under `.external-fixtures/`.
- Do not commit binary sample assets unless redistribution has been explicitly reviewed for this repository's MIT/Apache source distribution.
- Commit scripts, generated minimal JSON fixtures, and documentation only when they are source-like and license-safe.
