# TODO

This document is the current working backlog for `vrm-rs`. Keep detailed history in
`docs/progress.md`; keep this file focused on remaining work, priority, and done
criteria.

## How To Use

- Update this file when a task is completed, split, or superseded.
- Keep external binaries and generated golden files under `.external-fixtures/`; do not commit them.
- Before committing implementation work, run:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

## P0: Bevy Adapter Integration

- [x] Read real Bevy hierarchy into `BevyRuntimeSceneState`.
  - Done when `VrmNode` entities can derive parent/child links from Bevy ECS hierarchy components and spring/constraint/first-person tests no longer need manual `set_parent` setup.
- [x] Make `BevyVrmSpringParityState` recapture rules explicit.
  - Done when model changes, rest-pose changes, spring setup changes, and manual reset are represented by a small API or marker resource instead of requiring callers to remember `clear`.
- [x] Write `BevyVrmMaterialState` into renderer-facing material assets.
  - Done when at least one optional system maps material state into Bevy material assets while preserving the current render-feature-light default path.
- [x] Write morph weights into real Bevy mesh/skinned-mesh state.
  - Done when a Bevy app can drive expression morph targets without reading the lightweight staging component directly.
- [x] Implement Bevy first-person `auto` mesh handling.
  - Done when headless mesh planning can clone or update Bevy mesh assets for first-person view while keeping third-person meshes intact.

## P1: Renderer And MToon Practicality

- [ ] Add concrete MToon materialization examples for Bevy.
  - Done when base/outline pass plans, alpha/depth/cull state, render order, emissive strength, and texture refs are demonstrated against Bevy-facing assets or example components.
- [x] Add wgpu and ash integration skeleton examples.
  - Done when renderer owners can map `MtoonPipelinePass`, `MaterialRef`, and `TextureRef` into their own pipeline/material tables without depending on Bevy.
- [ ] Keep shader generation out of core crates.
  - Done when any renderer-specific shader work lives in examples, optional adapters, or downstream crates, with `vrm-core` and `vrm-adapter` remaining renderer-agnostic.

## P1: three-vrm Parity

- [ ] Deepen VRM0 numeric humanoid compatibility.
  - Done when Alicia or another VRM0 fixture has golden numeric checks for humanoid axis, rest pose, raw/normalized pose writeback, and legacy orientation behavior.
- [ ] Expand VRM0 legacy material edge cases.
  - Done when additional legacy material float/vector/texture-transform cases are covered by generated tests and at least one external fixture assertion.
- [ ] Improve spring bone numeric parity tolerance.
  - Done when the collider-heavy fixture tolerance is either tightened or the remaining float-path difference is documented with a focused solver test.
- [ ] Add more official spring fixture breadth.
  - Done when directory-level ignored golden tests cover more than Seed-san and the current collider-heavy constraint sample.
- [ ] Add fixture-driven node constraint manager parity.
  - Done when complete VRM scenes compare constraint ordering/writeback against three-vrm golden output, beyond standalone quaternion solver cases.
- [ ] Broaden VRMA parity.
  - Done when additional `.vrma` samples cover multiple clips and mixed humanoid rotation, hips translation, expression, and lookAt tracks.
- [ ] Tighten VRMA diagnostics.
  - Done when unsupported channel/path warnings and errors have stable tests and messages.

## P2: IO, Protocol, And Public API

- [ ] Raise `vrm-io` coverage.
  - Done when invalid GLB/glTF, buffer/image/accessor extraction, extension shape, and VRMA error paths are covered enough to materially raise line coverage from the current low-water mark.
- [ ] Expand protocol roundtrip coverage.
  - Done when representative unknown extension and `extras` retention tests exist for each major schema family.
- [ ] Polish the root facade.
  - Done when the root crate has an ergonomic load/validate/runtime-driver path for common `.vrm`, `.vrma`, `.gltf`, and `.glb` use.
- [ ] Add docs.rs-ready examples.
  - Done when facade, sans-IO, runtime driver, and adapter usage have short examples that compile in docs or integration tests.

## Ongoing Maintenance

- [ ] Keep `docs/progress.md` as a chronological log.
- [ ] Keep `docs/testing.md` coverage numbers current after coverage-affecting work.
- [ ] Keep `docs/adapter-guide.md` aligned with public adapter APIs.
- [ ] Ask a subagent for pessimistic review after larger parity or adapter slices.
