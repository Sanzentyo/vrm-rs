# TODO

This document is the current working backlog for `vrm-rs`. Keep detailed history in
`docs/progress.md`; keep this file focused on remaining work, priority, and done
criteria.

## How To Use

- Update this file when a task is completed, split, or superseded.
- Keep external binaries and generated golden files under `.external-fixtures/`; do not commit them.
- Before committing implementation work, run the local Rust CI script:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
```

The repository intentionally has no GitHub-hosted CI. Use `tools/ci/local-ci.rs` for the CI-equivalent local gate and its optional external fixture / render parity passes.

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

- [x] Add concrete MToon materialization examples for Bevy.
  - Done when base/outline pass plans, alpha/depth/cull state, render order, emissive strength, and texture refs are demonstrated against Bevy-facing assets or example components.
- [x] Add wgpu and ash integration skeleton examples.
  - Done when renderer owners can map `MtoonPipelinePass`, `MaterialRef`, and `TextureRef` into their own pipeline/material tables without depending on Bevy.
- [x] Keep shader generation out of core crates.
  - Done when any renderer-specific shader work lives in examples, optional adapters, or downstream crates, with `vrm-core` and `vrm-adapter` remaining renderer-agnostic.

## P1: three-vrm Parity

- [x] Deepen VRM0 numeric humanoid compatibility.
  - Done when Alicia or another VRM0 fixture has golden numeric checks for humanoid axis, rest pose, raw/normalized pose writeback, and legacy orientation behavior.
- [x] Expand VRM0 legacy material edge cases.
  - Done when additional legacy material float/vector/texture-transform cases are covered by generated tests and at least one external fixture assertion.
- [x] Improve spring bone numeric parity tolerance.
  - Done when the collider-heavy fixture tolerance is either tightened or the remaining float-path difference is documented with a focused solver test.
- [x] Add more official spring fixture breadth.
  - Done when directory-level ignored golden tests cover more than Seed-san and the current collider-heavy constraint sample.
- [x] Add fixture-driven node constraint manager parity.
  - Done when complete VRM scenes compare constraint ordering/writeback against three-vrm golden output, beyond standalone quaternion solver cases.
- [x] Broaden VRMA parity.
  - Done when additional `.vrma` samples cover multiple clips and mixed humanoid rotation, hips translation, expression, and lookAt tracks.
- [x] Tighten VRMA diagnostics.
  - Done when unsupported channel/path warnings and errors have stable tests and messages.

## P2: IO, Protocol, And Public API

- [x] Raise `vrm-io` coverage.
  - Done when invalid GLB/glTF, buffer/image/accessor extraction, extension shape, and VRMA error paths are covered enough to materially raise line coverage from the current low-water mark.
- [x] Expand protocol roundtrip coverage.
  - Done when representative unknown extension and `extras` retention tests exist for each major schema family.
- [x] Polish the root facade.
  - Done when the root crate has an ergonomic load/validate/runtime-driver path for common `.vrm`, `.vrma`, `.gltf`, and `.glb` use.
- [x] Add docs.rs-ready examples.
  - Done when facade, sans-IO, runtime driver, and adapter usage have short examples that compile in docs or integration tests.

## P3: Render Parity And External Automation

- [x] Add optional external-fixture local automation.
  - Done when a local Rust script downloads documented external fixtures into `.external-fixtures/official`, generates three-vrm golden files under `.external-fixtures/golden`, and runs ignored fixture/golden tests without committing binaries.
- [x] Expand redistributable official VRMA clip parity.
  - Done when fixture discovery records additional official VRMA clips, their license/provenance status, and semantic or golden parity assertions beyond the current `test.vrma` baseline where redistribution and CI use are acceptable.
  - Current discovery found stable `test.vrma` plus branch-only experimental `idle_loop.vrma`; both remain external-only, and `idle_loop.vrma` now extends hips translation scaling parity. See `docs/vrma-fixture-discovery.md`.
- [x] Add a non-Bevy engine adapter implementation.
  - Done when a concrete renderer-neutral or wgpu-oriented scene/material state implements the adapter traits enough for runtime-driver ticks, material writeback, and render-preparation examples without Bevy.
- [x] Deepen wgpu/ash material pipeline examples.
  - Done when examples map MToon descriptors into concrete pass ordering, alpha/depth/cull state, render order, texture/sampler bindings, uniform layouts, and pipeline/material keys for wgpu-style and ash-style renderers.
- [ ] Build the three-vrm render parity harness.
  - Done when three-vrm, Bevy, and wgpu render paths can produce comparable image artifacts, compute PSNR, store visual-review outputs under `.external-fixtures/`, and document thresholds used to judge compatibility.
  - First slices landed: `tools/render-parity/compare-psnr.mjs` and `docs/render-parity.md` define the RGBA artifact format, PSNR report, and initial visual-review thresholds; `tools/render-parity/three-vrm-browser-capture.mjs` captures the three-vrm WebGL reference frame through Chromium, flips WebGL readback rows into top-left order, and can write PNG review artifacts; `vrm-io` exposes mesh primitives, decoded image metadata, texture mappings, glTF PBR base-color texture fallback, and skin inputs for Bevy/wgpu buffer construction; `examples/wgpu_render_capture.rs` writes textured, rest-skinned wgpu RGBA/PNG artifacts from real mesh primitives and applies MToon-derived render order, cull, alpha, blend, depth-write policy, shade color, shading shift/toony, ambient, and effective emissive strength.
  - `tools/ci/local-ci.rs --render-parity` now regenerates three-vrm, wgpu, and Bevy RGBA artifacts under the same canonical transparent background contract, verifies that wgpu/Bevy alpha masks match the three-vrm reference within `--render-alpha-mismatch-tolerance`, writes canonical PNGs encoded from those RGBA artifacts, PSNR reports, amplified RGB/alpha diff heatmaps, and `.external-fixtures/render-parity/visual-review.html` in one local command. It defaults to `Seed-san.vrm`, accepts repeated `--render-fixture NAME.vrm` flags for broader official-sample sweeps, and applies `--render-fail-under` to an explicit `--render-psnr-metric` that defaults to `rgb-visible` for transparent captures. Direct `three-vrm-browser-capture.mjs --png-out` now also encodes PNG from raw RGBA readback data, so the standalone three-vrm preview path no longer depends on browser canvas compositing.
  - Bevy capture now has a Bevy 0.18.1 headless offscreen example using real mesh and texture inputs plus shared RGBA/PNG output. The Seed-san PSNR baseline is `28.25 dB` after rest-skinning, MToon-derived alpha/cull/render-order policy, matching the three-vrm/wgpu 30 degree camera projection, expanded outline meshes with outline width texture sampling, reference-exposure correction, glTF emissive factor/strength/texture handling, glTF metallic/roughness extraction with a compact GGX non-MToon fallback, base-plus-one outline ordering, a first custom Bevy MToon material/shader path with base, shade, shading-shift, matcap, rim, normal texture bindings, and lit outline-color mixing, Bevy-side generated tangents for normal-mapped primitives that omit glTF `TANGENT`, mipmapped repeat/linear texture sampling, configurable capture-only MToon lighting coefficients, explicit `Msaa::Off`, shared headless runtime world/skin matrices, double-sided back-face normal/TBN flipping, and resolved MToon inheritance of glTF base/emissive/texture/normal material params to match the antialias-disabled reference condition and adapter runtime path. The wgpu baseline is `28.42 dB` after matching three.js directional-light vector convention, adding expanded outline draws with outline width texture sampling and base-plus-one outline ordering, applying the same correction, binding MToon shade/matcap/shading-shift/rim/normal textures separately from the main texture, using tangent-space normal maps with generated tangents where needed and view-direction matcap UVs, consuming glTF alpha/double-sided/emissive/metallic/roughness material policy, flipping double-sided back-face normal/TBN, using the same compact GGX fallback for non-MToon glTF materials, matching three-vrm's rim/matcap composition and lit outline-color mixing more closely, adding mipmapped repeat/linear texture sampling, retuning the capture lighting defaults without changing the three-vrm reference, inheriting glTF base/emissive/texture/normal params into resolved MToon materials, and using the same shared headless runtime world/skin matrices as Bevy.
  - The broader official-sample render sweep covers `Seed-san.vrm`, `VRM1_Constraint_Twist_Sample.vrm`, `VRMC_materials_mtoon_UV_Animation_Test.vrm`, and the mask-heavy `VRMC_vrm_expressions_isBinary_{Overridden,Overrides}.vrm` samples; a separate `just render-parity-uv-animation` recipe advances MToon material time to `1.0` for UV scroll/rotation parity. Current transparent-background full-RGBA PSNR is Seed-san wgpu `28.42 dB`, Seed-san Bevy `28.25 dB`, constraint sample wgpu/Bevy `34.33 dB`, UV animation time `0` wgpu/Bevy `36.16 dB`, and UV animation time `1.0` wgpu/Bevy `35.92 dB`. The selected threshold metric is now `rgb-visible`, currently Seed-san wgpu `20.41 dB`, Seed-san Bevy `20.31 dB`, constraint sample wgpu/Bevy `25.65 dB`, UV animation time `0` wgpu/Bevy `27.45 dB`, and UV animation time `1.0` wgpu/Bevy `27.22 dB`. The first mask fixture smoke run reports zero partial-alpha pixels for wgpu/Bevy after alpha-test output normalization.
  - VRM0 MToon compatibility now follows more of three-vrm's v0 compat conversion for gamma-corrected colors, shade shift/toony normalization, GI equalization, centimeter outline width, outline lighting mode, alpha keywords, transparent Z-write, UV animation Y scroll, and transparent/transparent-Z-write render queue remapping.
  - Remaining parity blockers: exact MToon light/color accumulation, screen-coordinate outline fixture discovery/measurement, broader transparent material fixture render parity, and higher PSNR thresholds.

## Ongoing Maintenance

- [ ] Keep `docs/progress.md` as a chronological log.
- [ ] Keep `docs/testing.md` coverage numbers current after coverage-affecting work.
- [ ] Keep `docs/adapter-guide.md` aligned with public adapter APIs.
- [ ] Ask a subagent for pessimistic review after larger parity or adapter slices.
