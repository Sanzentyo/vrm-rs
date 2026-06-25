# AGENTS.md

Repository guidance for Codex sessions working on `vrm-rs`.

## Rust Work

- Before Rust implementation, testing, refactoring, or review work, read and follow the local `rust-best-practices` skill:
  - `C:\Users\sanze\.agents\skills\rust\SKILL.md`
- Before editing Rust code, Cargo workspace manifests, build scripts, tests, benches, or Rust documentation, read every explicitly relevant local Rust-related `SKILL.md`. If multiple Rust-related skills apply, summarize the applicable rules before implementation.
- Keep changes idiomatic Rust: no `unsafe`, no unstable features, no broad macro-heavy abstractions unless explicitly justified.
- Work in small, compiling increments. Prefer focused checks while developing, then run the normal local gate before implementation commits.
- Always run `cargo fmt`; use `cargo clippy --workspace --all-targets --all-features -- -D warnings` when feasible for the change scope.
- Scope tests to the changed surface during iteration instead of reflexively running every workspace test after every tiny edit. Add the broader gate before commits that affect shared behavior, adapters, IO, protocol, runtime paths, or coverage.
- New crates should include tests for their core behavior. Add snapshot/golden tests only when the output can be deterministic and license-safe.
- Run the normal gate before implementation commits through the local Rust CI script:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
```

- `just` is available as a convenience wrapper. Prefer `just ci` for the normal gate, while keeping the Rust script as the implementation source of truth:

```powershell
just ci
just ci-external
just ci-ash-windowed
just render-parity
just render-parity-samples
```

- The repository intentionally does not carry GitHub Actions workflows. `tools/ci/local-ci.rs` fails fast if `.github/workflows/*.yml` or `.github/workflows/*.yaml` files are present. Use the local Rust CI script when you want the old CI-equivalent gate:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --ash-windowed-smoke --ash-windowed-resize-smoke
cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity
```

The default script run covers `cargo fmt --all -- --check`, `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, non-rendering example smokes (`mtoon_renderer_skeletons`, `wgpu_mtoon_pipeline_materialization`, `ash_mtoon_pipeline_materialization`, `bevy_mtoon_materialization`, `custom_engine_adapter`), capture-example compile/unit tests (`wgpu_render_capture`, `bevy_render_capture`), render-tool syntax/self-tests, and `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70`.

## Rust API And Boundaries

- Public API must be intentional. Prefer typed APIs, newtypes, enums, and explicit boundary structs over stringly typed helpers or ad hoc string wrappers.
- When a workspace-owned enum or boundary type is missing behavior, add an inherent method on the owning type when the dependency direction permits it. Avoid scattering local extension traits, one-off match helpers, or duplicate wrapper functions across examples.
- Keep root-level `pub use` limited to deliberate facade API. In non-facade crates and modules, prefer explicit `pub mod` namespaces once multiple stable responsibilities emerge.
- Preserve the workspace layering: lower crates must not depend on higher crates. `vrm-core`, `vrm-protocol`, and `vrm-sans-io` stay renderer- and engine-neutral; filesystem, network, GPU, windowing, clock, and engine-specific behavior belongs in IO, runtime, adapter, example, or downstream crates as appropriate.
- Keep data-format and protocol crates Sans I/O. Path reads/writes, external fetches, signing, timing, and renderer/device resources should be adapter-side concerns.
- Backend-specific dependencies belong behind feature flags, adapter crates, examples, or downstream integrations. Do not leak Bevy, wgpu, ash, windowing, or shader implementation details into core/protocol layers.
- Prefer deterministic runtime behavior and structured errors. Use `thiserror` for workspace Rust error types unless there is a clear reason not to, and preserve structured fields such as `kind`, `range`, `anchor`, `path`, and `message` where diagnostics cross crate boundaries.

## Parser And Format Work

- Treat grammar docs and published format specifications as the source of truth.
- Prefer explicit AST/CST/schema nodes, source spans, and structured diagnostics over stringly typed parsing.
- Parser and format tests should cover successful inputs, malformed inputs, recovery/error spans, and ambiguity or compatibility rules for the full grammar family being touched.
- Add concise Rustdoc to public parser, schema, and AST-like types.
- For unfinished parser/compiler/data-format internals, prefer direct migration to the final model over compatibility shims. Do not add deprecated aliases, compatibility wrapper modules, or branches that silently accept removed syntax unless the user explicitly asks for a compatibility layer.

## Large Rust Files

- Split `lib.rs` and `main.rs` before they become catch-all implementation files. Keep facades small and move implementation into cohesive modules.
- For ordinary responsibility modules, treat roughly 300-800 LOC as a healthy target range. Larger modules are acceptable only when cohesion is clear; otherwise run a structure review before adding more code.
- In non-facade crates, avoid broad re-export surfaces as a substitute for module boundaries.

## Coverage Refresh Delegation

- Treat coverage table/progress updates as routine mechanical work suitable for delegation.
- Use a Codex worker running `gpt-5.4-mini` for routine coverage refreshes and other clearly specified mechanical edits.
- The `gpt-5.4-mini` worker must follow `docs/agents/coverage-mini.md` and use `tools/coverage/update-coverage-docs.ps1`.
- Preferred flow:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70 --json --output-path target/coverage-summary.json
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "YYYY-MM-DD"
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "YYYY-MM-DD" -Apply
git diff docs/testing.md docs/progress.md
```

- The first script run should be a dry run unless the user explicitly asked for direct application or the primary Codex turn has already reviewed the generated block.
- Keep `docs/testing.md` coverage snapshots and the newest relevant `docs/progress.md` coverage line synchronized.

## Primary Codex Responsibility

- The primary Codex turn still owns the implementation gate and final verification.
- Review delegated diffs and run gates before staging or committing; delegation does not transfer ownership of verification.
- Run `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70` locally before commits that affect tests, coverage, runtime paths, adapters, IO, or protocol behavior.
- If a `gpt-5.4-mini` worker updates coverage docs, review its diff before staging.
- If `gpt-5.4-mini` worker delegation is unavailable, perform the coverage refresh locally and record that limitation in the progress update.

## gpt-5.4 Mini Delegation

- Use a Codex worker with `gpt-5.4-mini` for routine mechanical work such as coverage refreshes and narrow bulk edits.
- For coverage refreshes and non-judgmental mechanical edits, prefer a `gpt-5.4-mini` worker when available.
- For pessimistic reviews, use the model and reasoning level requested by the user, and ask for findings focused on regressions, missing tests, and API hazards.
- Delegated workers should not make broad unrelated edits. Give them narrow ownership, ask them to report changed files, and review their output before integration.

## Fixtures And Licensing

- Keep official or third-party `.vrm`, `.vrma`, `.glb`, textures, and generated golden files under `.external-fixtures/`.
- Do not commit binary sample assets unless redistribution has been explicitly reviewed for this repository's MIT/Apache source distribution.
- Commit scripts, generated minimal JSON fixtures, and documentation only when they are source-like and license-safe.
