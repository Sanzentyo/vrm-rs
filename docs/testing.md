# Testing

## Generated Sample Data

The repository should not vendor third-party `.vrm`, `.vrma`, `.glb`, or texture assets for early tests. Instead, tests generate concrete minimal glTF JSON values in memory and feed them to the same public IO entrypoints used by real files.

Current generated coverage:

- Root `VRMC_vrm` 1.0 extension with meta, required humanoid bones, first-person annotations, lookAt, and expression binds.
- Root `VRMC_springBone` extension with collider, collider group, and spring joint data.
- Per-node `VRMC_node_constraint` extension.
- Per-material `VRMC_materials_mtoon` extension.
- Per-material archived `VRMC_materials_hdr_emissiveMultiplier` extension.
- Per-material `KHR_materials_emissive_strength` extension, including invalid shape handling, present-but-empty defaulting, and precedence over archived HDR multiplier.
- Invalid node reference, invalid extension shape, supported `1.0-beta`, and unsupported per-extension `specVersion` cases through the same generated sample.
- First-person headless mesh triangle erasure, humanoid pose snapshot diffing, spring extended collider `inside`, generated VRMA warning/error/application policy, MToon descriptor generation, Bevy adapter skeleton compile tests, and Bevy ECS hierarchy readback.

This keeps licensing simple while still exercising `gltf::import_slice`, extension extraction, sans-IO mapping, validation, and resolved model construction.

Later fixture strategy:

- Keep generated samples for unit and integration tests.
- Optional ignored tests read local user-provided assets recursively from `VRM_RS_FIXTURE_DIR`, defaulting to `.external-fixtures/official`.
- Do not commit proprietary or third-party avatar assets unless their license explicitly allows redistribution.

Run external fixture tests with:

```powershell
$env:VRM_RS_FIXTURE_DIR = (Resolve-Path ".external-fixtures/official")
cargo test -p vrm-io tests::loads_external_fixture_directory -- --ignored --exact
```

`.external-fixtures/` is ignored by git so official samples can be downloaded for local validation without becoming repository source assets.

The repository no longer carries GitHub Actions workflows. Use the local Rust script when a maintainer wants the old CI-equivalent gate:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
```

The root `Justfile` provides convenience wrappers while keeping the Rust script
as the single implementation of the gate:

```powershell
just ci
just ci-external
just render-parity
just render-parity-samples
just render-parity-samples-nonblack
just render-parity-vrm1-samples
```

The script intentionally fails before running the gate if `.github/workflows/*.yml` or `.github/workflows/*.yaml` is present. The default run is the local replacement for the removed hosted workflow: format check, workspace tests with all features, workspace clippy with warnings denied, non-rendering example smokes, and the conservative `cargo-llvm-cov` line threshold. The example smokes execute `mtoon_renderer_skeletons` and `bevy_mtoon_materialization`, so the public `MtoonRendererMaterialPlan` wgpu/ash-style path and Bevy-facing MToon material pipeline examples are checked by the normal local gate instead of only being compiled.

Run the external fixture parity pass locally with:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
```

That script downloads documented external fixtures into `.external-fixtures/official`, builds a pinned three-vrm checkout under `.external-fixtures/three-vrm`, regenerates golden JSON under `.external-fixtures/golden`, and runs the ignored fixture/golden tests without committing binaries. Fixture and golden environment variables should use absolute paths because Rust unit tests run with the package directory as their current directory.

Run the local render parity pass with:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity
```

This regenerates the Seed-san three-vrm, wgpu, and Bevy RGBA/PNG artifacts under `.external-fixtures/render-parity/`, writes PSNR reports under `.external-fixtures/render-parity/reports/`, creates diff heatmaps under `.external-fixtures/render-parity/diff/`, writes `.external-fixtures/render-parity/summary.md`, and creates `.external-fixtures/render-parity/visual-review.html` for side-by-side review. Use `--render-fail-under N` only after a renderer has reached a threshold that should be enforced.
Pass `--render-fixture NAME.vrm` more than once to broaden the render set while
keeping binaries external. `just render-parity-samples` currently renders
`Seed-san.vrm`, `VRM1_Constraint_Twist_Sample.vrm`, the official
`VRMC_materials_mtoon_UV_Animation_Test.vrm` fixture, the two official
`VRMC_vrm_expressions_isBinary_*` mask fixtures, and the external
`UniVRM/AliciaSolid_vrm-0.51.vrm` VRM0 transparent-material fixture, and
enforces selected `rgb-visible >= 32.5 dB`. Alicia is currently the lower bound
of that compatibility sweep at wgpu `32.6863 dB` / Bevy `32.6757 dB`, so
`just render-parity-vrm1-samples` keeps a stricter `34 dB` floor for the same
set without the VRM0 compatibility sample under
`.external-fixtures/render-parity-vrm1-samples/`. Because the canonical
opaque-black background makes every pixel visible, `rgb-visible` is the stable
full-review regression metric but can hide object-body color error behind black
background pixels. Use `just render-parity-samples-nonblack` for the same
six-fixture sweep under `.external-fixtures/render-parity-samples-nonblack/`;
it selects `rgb-nonblack >= 25 dB`, comparing only pixels where either render
has non-zero RGB. The current model-body floor is Alicia VRM0 at wgpu
`25.2374 dB` / Bevy `25.2268 dB`, while the same fixture reports
`rgb-nonblack-interior1px` around `27.88 dB` after dropping one-pixel edges.
Use
`--render-mtoon-time SECONDS` for MToon material-update parity checks such as UV
animation; `just render-parity-uv-animation` stores its time-advanced sample
under `.external-fixtures/render-parity-uv-animation/` so it does not overwrite
the canonical static sweep. Use `just render-parity-real-normal-maps` for the
focused real-fixture review of the known official tangentless normal-map
fixtures; it writes `.external-fixtures/render-parity-real-normal-maps/`.
Use `just render-parity-normal-maps-off` to disable normal maps across
three-vrm, wgpu, and Bevy when isolating whether a real-fixture delta comes from
tangentless normal-map behavior or another part of the material/geometry path.
Use `just render-parity-normal-maps-derivative` only as a diagnostic for the
shader-derivative tangent-frame fallback; the current measured path is worse
than generated tangents on Seed-san and is not the default guard.
Use `just render-parity-outline-off` as a diagnostic for separating MToon
outline expansion deltas from material, skinning, and pose deltas.
The canonical local runner now uses
`--render-background opaque-black`, so the three-vrm reference, wgpu capture,
and Bevy capture are all reviewed with the same opaque-background contract. Use
`--render-background transparent` only for explicit alpha-mask and silhouette
audits. Generated transparent-material guards currently include
`just render-parity-transparent-generated`,
`just render-parity-transparent-high-contrast`,
`just render-parity-transparent-broad`,
`just render-parity-transparent-texture-transform`,
`just render-parity-transparent-queue-matrix`, and
`just render-parity-transparent-alpha-modes`. Use
`just render-parity-transparent-lighted` for overlapping transparent layers
that also exercise MToon direct/shade/rim/emissive accumulation, and use
`just render-parity-transparent-depth-stack` for same-render-order BLEND layers
at different depths, including one texture-alpha layer; it keeps alpha buckets
exact while allowing only 1-LSB alpha channel rounding. The alpha-modes guard
specifically covers
OPAQUE alpha forcing, MASK `alphaCutoff` pass/fail behavior, and BLEND
`alphaCutoff` ignore behavior across three-vrm, wgpu, and Bevy. Generated
MToon material guards also include `just render-parity-mtoon-textures-generated`
for texture-slot parity and `just render-parity-mtoon-normal-generated` for the
current tangentless normal-map regression guard, which now enforces
`rgb-interior1px >= 45 dB`.
The local runner writes the three-vrm, wgpu, and Bevy PNGs from
their RGBA artifacts through the same Rust PNG encoder, so review images match
the exact buffers compared by PSNR. It decodes each PNG after writing and
requires a byte-for-byte match with the corresponding RGBA artifact, including
alpha. It also checks that the wgpu and Bevy alpha masks stay within
`--render-alpha-mismatch-tolerance` pixels of the three-vrm reference. The
render-parity run recreates its managed output directories first, so stale
direct-capture smoke PNGs are not mixed into the canonical review set. The
summary table lives at `.external-fixtures/render-parity/summary.md` and is also
embedded at the top of `visual-review.html`; use it as the first stop for
selected PSNR, max channel delta, alpha mismatch, and pass/fail status. The
compared images live
under `.external-fixtures/render-parity/three-vrm/`,
`.external-fixtures/render-parity/wgpu/`, and
`.external-fixtures/render-parity/bevy/`.
The runner keeps the reusable comparison logic Sans I/O where practical:
RGBA JSON parsing, alpha counting, diff heatmap pixel generation, PSNR report
summary extraction, and summary Markdown construction work on in-memory values,
while filesystem reads/writes remain in the surrounding runner functions.
The concrete wgpu and Bevy capture examples also share a backend-neutral
`CaptureMaterialPlan` alias over the public `RendererMaterialPipelinePlan` for
MToon/glTF alpha, culling, depth-write, blend, render-order, phase-order, and
transparent-order decisions; each renderer only converts that plan into its own
pipeline or material API at the edge.
If `tools/render-parity/three-vrm-browser-capture.mjs` is invoked directly
with `--png-out`, that PNG is also encoded from the raw RGBA readback buffer,
not from a browser canvas screenshot/data URL.
The three-vrm RGBA JSON additionally records `reference` metadata for the
Three.js revision, output color space, tone mapping, directional/ambient light
setup, alpha mode, and camera frustum so light/color parity reports can be
audited against the actual reference scene conditions. The generated MToon
light/color fixture now contains 12 swatches, including mid-ramp interpolation
cases, and the swatch comparator is run after aggregate PSNR to catch per-term
drift. For direct-light
isolation, `just render-parity-mtoon-light-direct-generated` disables ambient
on both the three-vrm and Rust capture sides while reusing the same generated
MToon light/color fixture. For ambient-light isolation,
`just render-parity-mtoon-light-ambient-generated` disables directional light
on both sides using the three-vrm directional intensity and Rust
`--render-direct-light-scale` controls.
The PSNR report additionally includes alpha counts/mismatches plus RGB-only
full-canvas, opaque, visible, and 1px-interior metrics to identify whether
remaining deltas come from silhouettes/alpha or from opaque-surface shading. When
`--render-fail-under N` is used, the local runner evaluates the selected
`--render-psnr-metric`, which defaults to `rgb-visible` for the visible surface
metric. Use `--render-psnr-metric rgba` for old full-buffer checks, or
`rgb-all` when alpha is validated separately but full-canvas RGB should still
be compared. Use `rgb-opaque`/`rgb-interior1px` when edge alpha disagreement
should be kept out of the threshold.
Use `rgb-visible-interior1px` for transparent-background audits that need
partial-alpha interiors included while still dropping one-pixel silhouette
edges. Use `rgb-nonblack` and `rgb-nonblack-interior1px` for opaque-black
diagnostics that should ignore empty black background pixels and focus on model
body color; these are most useful with `--render-background opaque-black`.

## Coverage

Line and branch coverage are measured with `cargo-llvm-cov`. The tool is not required for normal development, but release and parity work should use it when available.

Install:

```powershell
cargo install cargo-llvm-cov
```

Summary:

```powershell
cargo llvm-cov --workspace --all-features --summary-only
```

The local CI script runs the same workspace coverage pass with a conservative line threshold:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs
```

HTML report:

```powershell
cargo llvm-cov --workspace --all-features --html
```

Current known coverage gaps:

- Protocol roundtrip tests cover representative extension shapes, but not every optional schema field.
- External fixture tests assert semantic presence for official samples and compare Seed-san humanoid rest-state, posed humanoid writeback, spring center-space output, collider-heavy spring output, and VRMA application output against three-vrm.
- Runtime unit tests include representative three-vrm quaternion parity cases for node constraint rotation, roll, and aim solvers.
- Adapter tests use mock engines plus Bevy lightweight ECS systems and a renderer-agnostic wgpu/ash skeleton example; concrete Bevy render-asset writeback is still pending.
- Renderer-specific MToon shader generation is intentionally outside current coverage.
- Render parity is not yet fully satisfied across Rust renderers. P3 now has a PSNR comparator, RGBA artifact format, concrete three-vrm browser reference capture, textured wgpu offscreen capture, headless Bevy capture, UV-animation fixture coverage, mask-material fixture coverage, generated transparent-material guards, generated tangentless normal-map parity, direct/ambient isolated MToon light-color guards, a six-fixture real sweep gated at selected `rgb-visible >= 32.5 dB`, a matching object-body `rgb-nonblack >= 25 dB` diagnostic sweep, and a VRM1/current-official subset gated at `34 dB`; broader real-model PSNR, real tangentless normal-map fixture review, Alicia VRM0 model-body parity above the current floor, and higher final thresholds are still pending.

## Current Coverage Snapshot

Measured locally on 2026-05-31 with:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

| Scope | Region coverage | Line coverage |
| --- | ---: | ---: |
| Workspace total | 79.59% | 82.84% |
| `vrm-adapter-bevy` | 92.67% | 94.42% |
| `vrm-adapter` | 63.93% | 73.76% |
| `vrm-core` | 69.52% | 75.92% |
| `vrm-io` | 81.02% | 78.17% |
| `vrm-protocol` | 92.41% | 90.93% |
| `vrm-runtime` | 87.90% | 88.42% |
| `vrm-sans-io` | 92.69% | 95.68% |
| `facade src/lib.rs` | 98.77% | 100.00% |

The current external fixture tests cover recursive fixture discovery, semantic IO loading including the Alicia VRM0 compatibility sample, adapter spring rest capture/stepping, Seed-san center-space spring golden comparison, Alicia VRM0 spring golden comparison, Alicia VRM0 humanoid rest/writeback golden comparison, collider-heavy spring directory comparison, fixture-driven node constraint manager ordering/writeback comparison, Seed-san raw/normalized humanoid rest-state and posed writeback comparison, baseline plus dense Seed-san `test.vrma` application comparison, and branch-only `idle_loop.vrma` application comparison on real VRM/VRMA files without committing those binaries. Generated VRMA diagnostics cover stable warnings for ignored non-hips humanoid translation tracks, hips translation rest-height scaling, stable errors for invalid expression/lookAt animation paths, and normal-gate application of humanoid, preset/custom expression, and lookAt tracks through the adapter writeback path. Renderer-facing generated glTF coverage now also includes primitive `COLOR_0`, morph target deltas, mesh/node default morph weights, public MToon renderer material plans, and public primitive pipeline plans.

## Ordered Parity Milestones

The current ordered work queue is:

1. Posed humanoid writeback golden: generate deterministic `setRawPose` / `setNormalizedPose` three-vrm snapshots and compare Rust writeback against the resulting raw node transforms. Done for Seed-san.
2. Spring bone fixture expansion: add collider-heavy, center-node, and non-Seed-san official fixture golden comparisons while keeping external binaries out of git. Done for Seed-san plus VRM1_Constraint_Twist_Sample.
3. VRMA application parity: apply sampled VRMA frames to a model and compare humanoid rotations, hips translation, expression weights, and lookAt tracks against three-vrm behavior. Done for Seed-san plus stable `test.vrma` and branch-only `idle_loop.vrma`.

Latest completed ordered work queue:

1. VRMA parity breadth: compare normalized pose reconstructed from raw scene writeback as well as raw application output, assert that Rust does not emit unexpected expression keys, and support directory-level VRMA golden files for future fixture expansion.
2. VRM0 compatibility depth: add generated compatibility tests for legacy first-person flag spelling, lowercase fallback spellings, unknown flags, all lookAt range directions, and default range values.
3. Spring numeric precision: report and assert per-golden maximum tail/rotation component deltas so simple fixtures can remain tight while collider-heavy fixtures carry an explicit wider tolerance.

Latest completed implementation slice:

1. External fixture semantic breadth: assert known official fixture features for MToon UV animation, expression override samples, constraints, spring bones, and VRMA track classes.
2. VRMA fixture breadth: keep ignored directory tests ready for additional `.vrma` files by checking humanoid rotation, hips translation, expression, and lookAt track categories when present.
3. Bevy adapter skeleton: provide a minimal plugin/config marker as the first ECS entry point before concrete transform, morph, material, and mesh writeback systems are implemented.

Current active parity slice:

1. VRM0 external compatibility fixture: load UniVRM's Alicia VRM0 sample from `.external-fixtures/official/UniVRM/` and assert compatibility-level semantics without committing the binary asset.
2. Node constraint solver parity: port representative three-vrm rotation, roll, and aim quaternion cases into `vrm-runtime` unit tests.
3. Coverage refresh: rerun fmt/test/clippy/llvm-cov after the new assertions and update the snapshot if the totals change.

Latest VRM0 Alicia expansion:

1. Legacy expression aliases map into canonical expression keys (`aa`, `ih`, `ou`, `ee`, `oh`, `happy`, `sad`, `relaxed`, `lookUp`, `blinkLeft`, etc.).
2. Ignored IO fixture assertions now cover normalized humanoid aliases, first-person mesh annotations, Bone lookAt, VRM0 MToon material mapping, and VRM0 secondary animation conversion.
3. Renderer-facing MToon descriptors, `MtoonRendererMaterialPlan`, and `RendererMaterialPipelinePlan` now include the main VRM0/VRM1 materialization factors needed by Bevy/wgpu/ash adapters: base color, emissive factor, cutoff, receive shadow, shading grade, light attenuation, matcap, rim, outline lighting, texture-slot bindings, sampler hints, alpha/cull/depth/blend policy, and glTF alpha/double-sided overrides.
4. The Bevy adapter now has a `BevyMtoonMaterialPlan` conversion test that checks descriptor pass state, alpha/depth/cull state, base/shade/emissive colors, cutoff, texture references, and outline width without requiring Bevy render features.
5. The Bevy adapter now has a `BevyRuntimeSceneState` trait-implementation test that checks parent/child traversal, local/world transform synchronization, local translation writes, and visibility writes without enabling Bevy render/transform features.
6. The Bevy adapter now drives both high-level runtime paths in tests: `tick` for expression, first-person, MToon, and emissive writeback, and `tick_with_spring_parity` for `SpringRestMap` capture, center-space spring state, joint rotation writeback, and synchronized child world transforms.
7. The Bevy adapter scene state now records morph target weights, material color writes, texture transforms, emissive intensity writes, and MToon pipeline passes through the same adapter traits used by the runtime driver.
8. The Bevy adapter now has a `VrmRuntimeDriver` integration test that ticks the driver against `BevyRuntimeSceneState` and observes expression, MToon, emissive, and first-person visibility side effects.
9. The Bevy plugin now installs concrete ECS writeback systems tested through `App::update`, covering Bevy transform components plus lightweight visibility, morph-weight, and material-state components.
10. Bevy ECS readback is covered by a helper-system test that copies `VrmNode`, `Transform`, and `BevyVrmVisibility` components into `BevyRuntimeSceneState` for driver input staging.
11. Bevy runtime tick integration is covered by a full `App::update` path: read ECS transform state, run `VrmRuntimeDriver` from Bevy resources, then write expression, MToon, and emissive outputs back into lightweight Bevy components.
12. Bevy spring parity integration is covered by a full `App::update` path that reads ECS transforms, captures `SpringRestMap`, initializes center-space spring state, runs the runtime tick, and writes the solved joint rotation back to a Bevy `Transform`.
13. Bevy spring parity recapture is covered by a marker-resource test that requests a rest-pose recapture and verifies the captured `SpringRestMap` is rebuilt without callers manually clearing `BevyVrmSpringParityState`.
14. MToon renderer skeleton coverage now includes `cargo run --example mtoon_renderer_skeletons`, which maps public `MtoonRendererMaterialPlan` and `RendererMaterialPipelinePlan` values into wgpu-like and ash-like pipeline/material tables without renderer dependencies.
15. Bevy hierarchy readback now covers real `ChildOf` ECS hierarchy components, deriving `BevyRuntimeSceneState` parent/child links before spring parity and runtime-driver ticks.
16. Bevy MToon materialization coverage now includes `cargo run --example bevy_mtoon_materialization`, which maps MToon pass plans and runtime material state into a Bevy-facing asset without shader policy.

Each milestone should update this document before code changes, add ignored external-fixture commands when real assets are needed, keep generated golden JSON under `.external-fixtures/`, and run the normal fmt/test/clippy/coverage gate before commit.

## three-vrm Golden Generation

Build the local sibling `../three-vrm` workspace first, then generate spring golden output into the ignored fixture area:

```powershell
npx pnpm@10.24.0 install
npx pnpm@10.24.0 --filter @pixiv/three-vrm-springbone --filter @pixiv/three-vrm-core --filter @pixiv/three-vrm-materials-mtoon --filter @pixiv/three-vrm-materials-hdr-emissive-multiplier --filter @pixiv/three-vrm-materials-v0compat --filter @pixiv/three-vrm-node-constraint --filter @pixiv/three-vrm --filter @pixiv/three-vrm-animation build
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --three-vrm-root D:\git\three-vrm --frames 8 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\UniVRM\AliciaSolid_vrm-0.51.vrm --three-vrm-root D:\git\three-vrm --frames 4 --out D:\git\vrm-rs\.external-fixtures\golden\AliciaSolid_vrm-0.51.spring.json
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\VRM1_Constraint_Twist_Sample.vrm --three-vrm-root D:\git\three-vrm --frames 8 --out D:\git\vrm-rs\.external-fixtures\golden\VRM1_Constraint_Twist_Sample.spring.json
node tools\three-vrm-constraint-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\VRM1_Constraint_Twist_Sample.vrm --three-vrm-root D:\git\three-vrm --out D:\git\vrm-rs\.external-fixtures\golden\VRM1_Constraint_Twist_Sample.constraint.json
node tools\three-vrm-vrma-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --vrma D:\git\vrm-rs\.external-fixtures\official\test.vrma --three-vrm-root D:\git\three-vrm --times 0,0.5,1 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.test-vrma.json
node tools\three-vrm-vrma-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --vrma D:\git\vrm-rs\.external-fixtures\official\test.vrma --three-vrm-root D:\git\three-vrm --times 0,0.125,0.25,0.375,0.5,0.625,0.75,0.875,1 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.test-vrma-dense.json
node tools\three-vrm-vrma-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --vrma D:\git\vrm-rs\.external-fixtures\official\idle_loop.vrma --three-vrm-root D:\git\three-vrm --times 0,0.5,1 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.idle-loop-vrma.json
```

Run the ignored comparison with:

```powershell
$env:VRM_RS_THREE_VRM_GOLDEN = "D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json"
cargo test -p vrm-adapter tests::spring_parity_matches_three_vrm_golden_rotations -- --ignored --exact
cargo test -p vrm-adapter tests::humanoid_pose_matches_three_vrm_golden_rest_state -- --ignored --exact
cargo test -p vrm-adapter tests::humanoid_pose_writeback_matches_three_vrm_golden -- --ignored --exact
cargo test -p vrm-adapter tests::vrm0_alicia_humanoid_pose_matches_three_vrm_golden_rest_state -- --ignored --exact
cargo test -p vrm-adapter tests::vrm0_alicia_humanoid_pose_writeback_matches_three_vrm_golden -- --ignored --exact
$env:VRM_RS_THREE_VRM_GOLDEN_DIR = "D:\git\vrm-rs\.external-fixtures\golden"
cargo test -p vrm-adapter tests::spring_parity_matches_three_vrm_golden_directory -- --ignored --exact
cargo test -p vrm-adapter tests::node_constraint_manager_matches_three_vrm_golden -- --ignored --exact
$env:VRM_RS_THREE_VRM_VRMA_GOLDEN = "D:\git\vrm-rs\.external-fixtures\golden\Seed-san.test-vrma.json"
cargo test -p vrm-adapter tests::vrma_application_matches_three_vrm_golden -- --ignored --exact
$env:VRM_RS_THREE_VRM_VRMA_GOLDEN_DIR = "D:\git\vrm-rs\.external-fixtures\golden"
cargo test -p vrm-adapter tests::vrma_application_matches_three_vrm_golden_directory -- --ignored --exact
```

The golden output records public local rotations, three-vrm's private center-space spring tail state, humanoid raw/normalized rest/current poses, deterministic posed humanoid writeback scenarios, and VRMA application samples. The spring comparison checks center tails for all joints over multiple frames, including tiny-tail joints, and compares rotations only for stable-length joints. Extremely tiny tail vectors (`<= 0.001`) are skipped for quaternion comparison because their normalized direction is numerically sensitive, but their simulation state remains covered by the center-tail assertion. Spring tests now collect maximum tail and rotation component deltas per golden file; normal fixtures use `0.001` tail and `0.0015` rotation tolerance, while collider-heavy constraint fixtures use `0.003` tail and `0.0015` rotation tolerance because collider resolution accumulates small three.js/Rust float-path differences over chained joints. VRMA application parity compares raw humanoid pose after normalized-to-raw writeback, normalized pose reconstructed from the written raw scene, expression weights without allowing unexpected Rust-only keys, and lookAt quaternion at deterministic sample times.

## Current External Official Samples

Spark downloaded the current local fixture set into `.external-fixtures/official/` on 2026-04-29. These files are intentionally ignored by git.
VRMA clip discovery is tracked in `docs/vrma-fixture-discovery.md`; as of the
latest check, `test.vrma` is the stable public upstream `.vrma` sample and
`idle_loop.vrma` is an experimental branch-only upstream sample used for
external-only parity breadth.

| File | Source | Local use note |
| --- | --- | --- |
| `Seed-san.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/Seed-san/vrm/Seed-san.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `VirtualCast, Inc.`; keep external because it is not MIT/Apache source code. |
| `VRM1_Constraint_Twist_Sample.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; keep external. The three-vrm mirror is byte-identical. |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_materials_mtoon_UV_Animation_Test/vrm/VRMC_materials_mtoon_UV_Animation_Test.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for MToon UV animation parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overridden/vrm/VRMC_vrm_expressions_isBinary_Overridden.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overrides/vrm/VRMC_vrm_expressions_isBinary_Overrides.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `UniVRM/AliciaSolid_vrm-0.51.vrm` | `https://raw.githubusercontent.com/vrm-c/UniVRM/cc52748645889e1521f5a4cef2103b8b028100bf/Tests/Models/Alicia_vrm-0.51/AliciaSolid_vrm-0.51.vrm` | VRM0 compatibility fixture for ignored semantic tests. Keep external until redistribution/license status is reviewed for this repository's MIT/Apache source distribution. |
| `test.vrma` | `https://raw.githubusercontent.com/pixiv/three-vrm/9d125586f6d7da094b0ac5f204cebf19586f2397/packages/three-vrm-animation/examples/models/test.vrma` | Local testing only until upstream redistribution status is confirmed; no embedded asset license/provenance found. |
| `idle_loop.vrma` | `https://raw.githubusercontent.com/pixiv/three-vrm/75ab65c9d4e488521d41bff7f5cfd1976a0b16e8/packages/vrm-viewer/examples/models/idle_loop.vrma` | Experimental branch-only three-vrm viewer clip. Useful for hips translation scaling parity. External only; do not vendor without explicit review. |

The URLs are commit-pinned to avoid branch drift.
