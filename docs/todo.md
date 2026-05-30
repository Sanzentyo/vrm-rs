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
  - `tools/ci/local-ci.rs --render-parity` now regenerates three-vrm, wgpu, and Bevy RGBA artifacts under the same canonical opaque-black visual-review background contract, while `--render-background transparent` remains available for alpha-mask audits. It verifies that wgpu/Bevy alpha masks match the three-vrm reference within `--render-alpha-mismatch-tolerance`, writes canonical PNGs encoded from those RGBA artifacts, PSNR reports, amplified RGB/alpha diff heatmaps, and `.external-fixtures/render-parity/visual-review.html` in one local command. It defaults to `Seed-san.vrm`, accepts repeated `--render-fixture NAME.vrm` flags for broader official-sample sweeps, and applies `--render-fail-under` to an explicit `--render-psnr-metric` that defaults to `rgb-visible`. Direct `three-vrm-browser-capture.mjs --png-out` now also encodes PNG from raw RGBA readback data, so the standalone three-vrm preview path no longer depends on browser canvas compositing.
  - Bevy capture now has a Bevy 0.18.1 headless offscreen example using real mesh and texture inputs plus shared RGBA/PNG output. The Seed-san PSNR baseline is `28.25 dB` after rest-skinning, MToon-derived alpha/cull/render-order policy, matching the three-vrm/wgpu 30 degree camera projection, expanded outline meshes with outline width texture sampling, reference-exposure correction, glTF emissive factor/strength/texture handling, glTF metallic/roughness extraction with a compact GGX non-MToon fallback, base-plus-one outline ordering, a first custom Bevy MToon material/shader path with base, shade, shading-shift, matcap, rim, normal texture bindings, and lit outline-color mixing, Bevy-side generated tangents for normal-mapped primitives that omit glTF `TANGENT`, mipmapped repeat/linear texture sampling, configurable capture-only MToon lighting coefficients, explicit `Msaa::Off`, shared headless runtime world/skin matrices, double-sided back-face normal/TBN flipping, and resolved MToon inheritance of glTF base/emissive/texture/normal material params to match the antialias-disabled reference condition and adapter runtime path. The wgpu baseline is `28.42 dB` after matching three.js directional-light vector convention, adding expanded outline draws with outline width texture sampling and base-plus-one outline ordering, applying the same correction, binding MToon shade/matcap/shading-shift/rim/normal textures separately from the main texture, using tangent-space normal maps with generated tangents where needed and view-direction matcap UVs, consuming glTF alpha/double-sided/emissive/metallic/roughness material policy, flipping double-sided back-face normal/TBN, using the same compact GGX fallback for non-MToon glTF materials, matching three-vrm's rim/matcap composition and lit outline-color mixing more closely, adding mipmapped repeat/linear texture sampling, retuning the capture lighting defaults without changing the three-vrm reference, inheriting glTF base/emissive/texture/normal params into resolved MToon materials, and using the same shared headless runtime world/skin matrices as Bevy.
  - The broader official-sample render sweep covers `Seed-san.vrm`, `VRM1_Constraint_Twist_Sample.vrm`, `VRMC_materials_mtoon_UV_Animation_Test.vrm`, the mask-heavy `VRMC_vrm_expressions_isBinary_{Overridden,Overrides}.vrm` samples, and the external `UniVRM/AliciaSolid_vrm-0.51.vrm` VRM0 transparent-material fixture; a separate `just render-parity-uv-animation` recipe advances MToon material time to `1.0` for UV scroll/rotation parity. The capture light vector now follows three-vrm's directional-light direction through the VRM orientation compensation, and `just render-parity-samples` enforces selected `rgb-visible >= 32 dB`. Current selected PSNR is Seed-san wgpu `32.8751 dB`, Seed-san Bevy `32.5273 dB`, constraint sample wgpu `35.9456 dB`, constraint sample Bevy `35.9394 dB`, UV animation wgpu `34.8985 dB`, UV animation Bevy `34.8819 dB`, mask samples around `39.29-39.30 dB`, and Alicia VRM0 wgpu/Bevy `32.34 dB`. All six fixtures now report fully opaque alpha and zero alpha mismatches under the canonical review background.
  - `just render-parity-real-transparent` covers the same six real external fixtures on transparent background. It now uses selected `rgb-all` PSNR with fail-under `32`, while alpha mismatches are checked separately with tolerance `64`: Seed-san wgpu/Bevy `25/32`, constraint `11/11`, UV animation `0/0`, expression mask samples `12/12`, Alicia VRM0 `32/32`. Current selected PSNR passes that floor: Seed-san wgpu `32.8751 dB`, Bevy `32.5273 dB`; constraint wgpu `35.9456 dB`, Bevy `35.9394 dB`; UV animation wgpu `34.8985 dB`, Bevy `34.8819 dB`; expression masks around `39.29-39.30 dB`; Alicia wgpu `32.3424 dB`, Bevy `32.3410 dB`.
  - License-safe generated transparent fixtures are available through `just render-parity-transparent-generated`, `just render-parity-transparent-high-contrast`, `just render-parity-transparent-broad`, `just render-parity-transparent-alpha-modes`, and `just render-parity-transparent-depth-stack`. They write source-like VRM1 glTF files under `.external-fixtures/generated/`, render on a transparent background, and currently verify exact alpha-bucket parity for overlapping MToon `BLEND` primitives (`transparent=512 opaque=0 partial=65024`) across three-vrm, wgpu, and Bevy. The normal fixture verifies transparent RGB accumulation with an embedded bufferView PNG base-color texture: wgpu reports selected `rgb-visible = 53.0238 dB` with max channel delta `1`, and Bevy reports `49.7151 dB` with max channel delta `2`. The high-contrast palette stresses equal-depth layer ordering and now reports wgpu `53.1994 dB` with max channel delta `1`, and Bevy `51.9341 dB` with max channel delta `2`, after Bevy's capture path gained a `Transparent3d` phase-order tie-break. The broad fixture adds four overlapping layers, mixed render queue offsets, `transparentWithZWrite`, and texture-driven alpha; it passes with no alpha deltas beyond 1 LSB, selected `rgb-visible` PSNR wgpu `48.5282 dB` and Bevy `48.5944 dB`. The depth-stack fixture adds three same-render-order BLEND layers at different depths plus one texture-alpha layer; it passes with exact alpha buckets (`transparent=31672`, `opaque=0`, `partial=33864`), no alpha deltas beyond 1 LSB, selected `rgb-visible` PSNR wgpu `49.9331 dB` and Bevy `51.8518 dB`. The alpha-mode fixture covers OPAQUE alpha forcing, MASK custom cutoff pass/fail behavior, and BLEND cutoff-ignore behavior; it passes with exact alpha buckets (`transparent=37904`, `opaque=19184`, `partial=8448`), zero alpha mismatches, selected `rgb-visible` PSNR wgpu `47.4970 dB` and Bevy `48.0126 dB`. The fixture includes `COLOR_0` to guard the spec/three-vrm behavior that MToon ignores vertex colors while renderer IO still extracts them for non-MToon materials.
  - VRM0 MToon compatibility now follows more of three-vrm's v0 compat conversion for gamma-corrected colors, shade shift/toony normalization, GI equalization, centimeter outline width, outline lighting mode, alpha keywords, transparent Z-write, UV animation Y scroll, transparent/transparent-Z-write render queue remapping, and the `V0_COMPAT_SHADE` direct-light clamp.
  - wgpu/Bevy MToon shader policy now avoids the incorrect main-texture fallback for missing `shadeMultiplyTexture` and applies matcap texture transforms to sphere UVs like three-vrm.
  - `--mtoon-light-accumulation three-vrm` now uses the reference-shaped exposure `1.0`, ambient irradiance `pbrAmbient`, no GI ambient proxy, and direct-plus-indirect rim accumulator; `tuned` remains the default for the broader PSNR sweep.
  - A source-like generated MToon light/color fixture is available through `just render-parity-mtoon-light-generated`. It covers forced base, forced shade, three-vrm-compatible ignored MToon `occlusionTexture`, parametric rim, matcap rim, mixed rim/matcap, angled-normal toon-ramp quads, pure `emissiveFactor`, and `emissiveTexture` multiplied by `KHR_materials_emissive_strength` without committing binary assets. The recipe uses transparent background plus `rgb-interior1px` so the selected score measures swatch interior color rather than the known raster edge delta; current selected PSNR is wgpu `58.9968 dB` and Bevy `51.8032 dB`, with max selected channel deltas `1` and `2`. `just render-parity-mtoon-light-direct-generated` disables ambient on both sides and still passes at wgpu `58.9803 dB` / Bevy `51.7674 dB`, keeping direct-light color parity separate from the default ambient `0.1` reference setup. `just render-parity-mtoon-light-ambient-generated` disables directional light on both sides through the three-vrm reference intensity and Rust direct-light scale controls, and passes at wgpu `60.9964 dB` / Bevy `53.7214 dB`.
  - A source-like generated MToon texture-slot fixture is available through `just render-parity-mtoon-textures-generated`. It covers `shadingShiftTexture`, `rimMultiplyTexture`, `uvAnimationMaskTexture`, and `outlineWidthMultiplyTexture` at `mtoon-time=1.0` with exact alpha parity and selected `rgb-interior1px` PSNR wgpu `53.5556 dB` / Bevy `50.9425 dB`.
  - A separate opt-in generated MToon normal-map fixture is available through `just render-parity-mtoon-normal-generated`. It adds a tangentless normal-textured MToon swatch without committing binary assets, passes exact alpha parity, and currently reports selected `rgb-interior1px` PSNR wgpu `47.1013 dB` / Bevy `46.0637 dB`. The recipe now enforces `rgb-interior1px >= 45 dB`. The three-vrm reference rejected the synthetic normal swatch when a `TANGENT` attribute was present with a `vTangent` WebGL validation error, so this fixture intentionally exercises the tangentless fallback path; the Bevy path now preserves generated tangents for referenced vertices even when the accessor includes unreferenced vertices.
  - A source-like generated screen-coordinate outline fixture is available through `just render-parity-screen-outline-generated`. It covers `outlineWidthMode = screenCoordinates` against three-vrm/wgpu/Bevy with selected `rgb-opaque` PSNR wgpu `Infinity` and Bevy `53.2689 dB`; the known one-pixel fill-rule alpha edge mismatch is tracked separately (`188`, tolerance `256`).
  - `tools/render-parity/inspect-mtoon-fixtures.rs` and `just inspect-mtoon-fixtures` now inventory MToon feature coverage from local fixtures without committing binary samples. The current official fixture set contains no screen-coordinate outline materials, but does cover world outline, transparent-ZWrite, UV animation, normal textures, matcap textures, and shade textures; generated fixtures provide the missing screen-outline, strict transparent-ZWrite, texture-slot, alpha-mode, and tangentless normal-map guards. The scanner also shows `21` official normal-mapped primitives without glTF `TANGENT`, making three-vrm's derivative normal-map fallback a concrete follow-up candidate; a naive derivative attempt was measured and rejected after Seed-san fell to `20.3102 dB`.
  - Remaining parity blockers: runtime/material breadth on broader real model fixtures beyond the covered reference-shaped accumulator, real tangentless normal-map fixture review, real transparent-fixture visual parity beyond the current regression floors, finding or legally adding a real screen-coordinate outline fixture if one becomes available beyond the generated guard, and higher PSNR thresholds.

## Ongoing Maintenance

- [ ] Keep `docs/progress.md` as a chronological log.
- [ ] Keep `docs/testing.md` coverage numbers current after coverage-affecting work.
- [ ] Keep `docs/adapter-guide.md` aligned with public adapter APIs.
- [ ] Ask a subagent for pessimistic review after larger parity or adapter slices.
