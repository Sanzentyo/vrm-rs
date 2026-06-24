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
- Dependency-free OSC packet roundtrips for all supported OSC value variants, nested arrays/bundles, UDP packets, TCP length-prefixed packets, stream-fragment waiting, OSC string padding, and root facade `osc` feature re-export.
- Typed VMC 3.1 packet coverage for Marionette/Performer motion, camera/light, controller/key/MIDI input, device pose, receive/config/VRM/remote/settings/window/period/eye/calibration/shortcut messages, `/VMC/Thru/*` passthrough, official and legacy camera/light address handling, strict parse-before-apply transactions, rollback on sink errors, lenient invalid-known-message skipping, and socket-free transport policy gates for sender allow lists, per-sender rate limits, packet message limits, relative-time rewind/jump checks, and all-or-nothing runtime sink application.
- `vrm-io` optimizer preprocessing for degenerate triangle removal, unused vertex compaction, stale unused joint data, skin-weight normalization, weighted joint-palette compaction, empty morph target removal, skin palette application, and invalid attribute/index/joint diagnostics.
- `vrm-io` codec/resource registry behavior for data URI and relative-file path safety, missing codec errors, decoded-size limits, option-aware KTX2/Basis-style texture provider dispatch, source color-space propagation, renderer GPU format capability selection, and unsupported decoded texture format rejection.
- Full metadata source-preservation coverage for `GltfSource::vrm_full_metadata`, including VRM1 typed author/license fields plus meta extensions/extras and unknown raw fields, VRM0 legacy author/license/permission fields plus unknown raw fields, missing/malformed metadata errors, and root facade `VrmFullMetadata` re-export.
- VRM0 LookAt compatibility for Unity-style two-key Hermite `FirstPersonDegreeMap.curve` preservation, default-linear curve behavior, non-linear curve evaluation, and runtime expression LookAt weight output through the curve-aware range mapper.
- Policy-aware `vrm-io` loading with structured diagnostics: strict loaders keep fail-fast behavior, lenient loading reports malformed VRM1 expression JSON with a stable path and skips that expression, unknown root extensions are preserved in `GltfSource` while reported as warnings, and existing VRMA/animation warnings are mirrored into `DiagnosticReport`.
- Preserved-source writer coverage for compact/pretty `.gltf` and `.glb` JSON output, VRM1/VRM0 metadata patch helpers, same-directory atomic save failure behavior, GLB declared-length and chunk-alignment validation, and unknown GLB chunk retention through edits.

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
just render-parity-with-ash-readback
just render-parity-samples
just render-parity-samples-nonblack
just render-parity-vrm1-samples
just render-parity-imqraw-seed-normal
```

Recommended stable entrypoints:

| Task | Command |
| --- | --- |
| Normal local gate | `just ci` |
| External fixture/golden refresh | `just ci-external` |
| Headless VRMA sampling smoke | `just vrma-animation` |
| Bevy release viewer | `just bevy-vrma-viewer` |
| Bevy-independent wgpu release viewer | `just wgpu-vrma-viewer` |
| Browser/WASM viewer compile check | `just wasm-web-check` |
| Bevy browser viewer release wasm-pack build | `just wasm-bevy-web-build` |
| wgpu browser viewer release wasm-pack build | `just wasm-wgpu-web-build` |
| ash frame-plan smoke without Vulkan device setup | `just ash-vrma-frame-plan` |
| ash renderer-edge mock integration | `just ash-renderer-integration` |
| ash WGSL-to-SPIR-V MToon naga probe | `just ash-mtoon-naga-probe` |
| ash WGSL/Naga MToon readback smoke | `just ash-mtoon-base-readback` |
| ash real Vulkan offscreen submit/readback smoke | `just ash-unsafe-device-renderer` |
| ash real Vulkan window/swapchain smoke | `just ash-windowed-viewer-smoke` |
| ash windowed cache hit validation smoke | `just ash-windowed-viewer-cache-smoke` |
| ash windowed resize/swapchain recreation smoke | `just ash-windowed-viewer-resize-smoke` |
| ash opt-in windowed local CI lane | `just ci-ash-windowed` |
| ash readback artifact smoke | `just ash-render-parity-readback` |
| Current official sample render sweep | `just render-parity-samples` |
| Seed-san render sweep plus supplemental ash readback artifacts | `just render-parity-with-ash-readback` |
| Real transparent-background render sweep | `just render-parity-real-transparent` |
| Raw imqraw Seed-san normal-map cross-check | `just render-parity-imqraw-seed-normal` |

The many focused `render-parity-*diagnostic`, generated-fixture, hotspot, and
owner-id recipes are intentionally kept in the Justfile as investigation tools.
They are not the normal compatibility gate unless a task or regression points at
that specific slice.

The script intentionally fails before running the gate if `.github/workflows/*.yml` or `.github/workflows/*.yaml` is present. The default run is the local replacement for the removed hosted workflow: format check, workspace tests with all features, workspace clippy with warnings denied, non-rendering example smokes, capture-example compile/unit tests, render-tool syntax/self-tests, and the conservative `cargo-llvm-cov` line threshold. The example smokes execute `mtoon_renderer_skeletons`, `wgpu_mtoon_pipeline_materialization`, `ash_mtoon_pipeline_materialization`, `bevy_mtoon_materialization`, `custom_engine_adapter`, `headless_vrma_animation --help`, `cargo run --release --example bevy_vrma_viewer -- --help`, `cargo run --release -p vrm-adapter-wgpu --example vrma_viewer -- --help`, `cargo run --release -p vrm-adapter-ash --example frame_plan -- --help`, `cargo run --release -p vrm-adapter-ash --example renderer_integration -- --help`, `cargo run --release -p vrm-adapter-ash --example unsafe_device_renderer -- --help`, and `cargo run --release -p vrm-adapter-ash --example unsafe_device_renderer -- --artifact-self-test`, so the renderer-neutral skeleton, concrete wgpu-shaped bind-group/render-pipeline mapping, concrete Vulkan-shaped separate sampled-image/sampler descriptor mapping, Bevy-facing MToon material pipeline example, non-Bevy custom-engine runtime adapter flow, renderer-neutral VRMA animation CLI, release-built Bevy viewer entrypoint, release-built Bevy-independent wgpu viewer entrypoint, release-built ash/Vulkan frame-plan entrypoint, release-built ash renderer-edge integration entrypoint, real ash offscreen drawable materialization entrypoint, and ash RGBA/imqraw artifact writers stay checked by the normal local gate instead of only being compiled. `cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --ash-windowed-smoke --ash-windowed-resize-smoke` is the opt-in GPU/window lane for the Ash swapchain viewer; `just ci-ash-windowed` wraps that command and runs the cache-hit and resize-recreate smokes through the Rust script rather than treating the direct viewer recipes as the CI source of truth. `just ash-mtoon-base-readback` compiles the source-controlled Ash WGSL shader through naga, feeds the resulting SPIR-V through `unsafe_device_renderer --descriptor-binding-model separate-image-sampler --vertex-spv --fragment-spv --vertex-entry vs_main --fragment-entry fs_main`, and verifies the external-shader readback artifacts. `just ash-mtoon-glsl-base-readback` remains available for explicit legacy combined-sampler GLSL checks. The environment-dependent `just ash-unsafe-device-renderer` recipe goes further than CI by submitting the recorded offscreen draw and reading back the color attachment checksum; `just ash-render-parity-readback` also writes `.rgba.json` and direct `.imqraw` artifacts and verifies that the raw bundle matches the JSON bytes. `just render-parity-with-ash-readback` now also compiles that source shader inside the render-parity runner, emits Ash comparison reports and diff heatmaps beside wgpu/Bevy, and leaves Ash non-gating unless `--render-ash-visual-gate` is explicitly passed. The same default gate also runs `cargo test --example wgpu_render_capture --all-features`, `cargo test --example bevy_render_capture --all-features`, `node --check` for the three-vrm browser capture script, and the Rust render-tool help/self-test commands used by the parity harness.
The Rust script also sets cargo dev/test debug info to level `1` for commands
it launches. That keeps Windows MSVC PDB files below the observed Bevy-heavy
debug-info limit without changing runtime behavior, render output, or the
coverage threshold.

Run the external fixture parity pass locally with:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
```

That script downloads documented external fixtures into `.external-fixtures/official`, builds a pinned three-vrm checkout under `.external-fixtures/three-vrm`, regenerates golden JSON under `.external-fixtures/golden`, and runs the ignored fixture/golden tests without committing binaries. Fixture and golden environment variables should use absolute paths because Rust unit tests run with the package directory as their current directory.

Run the local render parity pass with:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity
```

This regenerates the Seed-san three-vrm, wgpu, and Bevy RGBA/PNG artifacts under `.external-fixtures/render-parity/`, writes PSNR reports under `.external-fixtures/render-parity/reports/`, creates diff heatmaps under `.external-fixtures/render-parity/diff/`, writes `.external-fixtures/render-parity/summary.md`, creates `.external-fixtures/render-parity/visual-review.html` for side-by-side review, and writes `.external-fixtures/render-parity/review-manifest.json` as a machine-readable audit index. Use `--render-fail-under N` only after a renderer has reached a threshold that should be enforced.
The run validates that manifest before returning success. To re-check a
previous artifact set, run `just render-parity-validate
.external-fixtures/render-parity/review-manifest.json`.
The three-vrm, wgpu, and Bevy captures also write `.frame000.imqraw` files
beside their `.rgba.json` artifacts. Check them with
`imq bundle-info PATH --format json` or pipe the bundle into
`imq image - - --stdin-format imqraw` until the installed CLI auto-detects
`.imqraw` path arguments.
`tools/render-parity/compare-imqraw.rs` compares those direct bundles with the
same VRM domains as `compare-psnr.mjs`. The render parity runner uses the
`.imqraw-rust.json` reports as the numeric gate and still writes `.psnr.json`
reports as `.rgba.json` diagnostics.
The direct imqraw report also includes a `changedPixels` section with RGB/RGBA
changed-pixel counts, expected-only/actual-only nonblack pixels,
shared-nonblack interior bands at 1/2/3px, flat32/gradient interiors, and
`highDelta` buckets for max channel deltas `>=32`, `>=64`, `>=96`, and
`>=128`. High-delta buckets split expected-only, actual-only, combined
coverage-only, and shared-nonblack pixels before the shared-nonblack edge and
flat/gradient interior breakdowns. Fields named `*Rgb` are scoped to pixels
whose RGB channels changed and therefore exclude alpha-only drift; edge-band
ratios are also reported as `*RatioOfSharedNonblackRgb` to make their
denominator explicit. Use those fields to distinguish one-sided
coverage/ownership regressions from broad material/color regressions, local
raster edge deltas, or dense-gradient ownership residuals before changing shader
logic.
It also runs `tools/render-parity/verify-imqraw-rgba.rs` for each three-vrm,
wgpu, and Bevy capture, so the numeric-gate `.imqraw` bytes must match the
`.rgba.json` bytes used for PNGs, diff heatmaps, and diagnostic reports.
Pass `--render-ash-readback` together with `--render-parity` when you want the
ash unsafe offscreen renderer to emit supplemental `.rgba.json`, `.imqraw`, and
PNG artifacts under the same render-parity directory. Those ash artifacts are
validated with the same imqraw/RGBA byte check and recorded in
`review-manifest.json`; when `--render-ash-visual-gate` is also passed, Ash is
checked by the same selected-PSNR and alpha-consistency gates as wgpu and Bevy.
`just render-parity-samples-ash-gated` is the current opaque-black six-fixture
smoke for that path, and `just render-parity-real-transparent-ash-gated` runs
the matching transparent-background six-fixture gate. For source-like generated
transparent-material coverage, the generated transparent `*-ash-gated` recipes
add Ash to the same visual gate as wgpu and Bevy for base BLEND, high-contrast
BLEND, broad BLEND, texture-transform, queue-matrix, alpha-mode, mask-texture,
depth-stack, and lighted MToon cases.
For a PNG-free cross-check of existing RGBA artifacts, use
`just imqraw-compare-rgba EXPECTED.rgba.json ACTUAL.rgba.json REPORT.json` or
the focused `just render-parity-imqraw-seed-normal` recipe. That path uses the
`imqraw` TypeScript/WASM library to pack `.rgba.json` buffers into a lossless
stdin bundle and lets `imq image` compute metrics directly from the raw RGBA
data.
Pass `--render-fixture NAME.vrm` more than once to broaden the render set while
keeping binaries external. `just render-parity-samples` currently renders
`Seed-san.vrm`, `VRM1_Constraint_Twist_Sample.vrm`, the official
`VRMC_materials_mtoon_UV_Animation_Test.vrm` fixture, the two official
`VRMC_vrm_expressions_isBinary_*` mask fixtures, and the external
`UniVRM/AliciaSolid_vrm-0.51.vrm` VRM0 transparent-material fixture, and
enforces selected `rgb-visible >= 34 dB`. Seed-san Bevy is currently the lower
bound of that compatibility sweep at `34.0835 dB`, while Alicia VRM0 now
reports wgpu `35.6238 dB` / Bevy `35.6088 dB` after sampler-policy and
CatmullRom mip-chain parity.
`just render-parity-vrm1-samples` keeps a separate `34 dB` floor for the same
set without the VRM0 compatibility sample under
`.external-fixtures/render-parity-vrm1-samples/`. Because the canonical
opaque-black background makes every pixel visible, `rgb-visible` is the stable
full-review regression metric but can hide object-body color error behind black
background pixels. Use `just render-parity-samples-nonblack` for the same
six-fixture sweep under
`.external-fixtures/render-parity-samples-nonblack-interior/`; use
`just render-parity-samples-nonblack-ash-gated` when Ash should join the same
gate under `.external-fixtures/render-parity-samples-nonblack-ash-gated/`.
Both select `rgb-nonblack-interior1px >= 27.4 dB`, comparing model-body pixels
where either render has non-zero RGB while dropping the one-pixel silhouette
edge. The current Ash-gated model-body floor is Seed-san Bevy `29.4512 dB`;
Seed-san wgpu is `30.1229 dB` and Ash `30.1042 dB`, constraint is around
`29.72-29.74 dB`, UV animation is around `29.56-29.61 dB`, expression samples
are above `50 dB`, and Alicia VRM0 is wgpu/Ash `34.4442 dB` / Bevy
`34.3709 dB`.
Use `just render-parity-samples-shared-body3` for the same six-fixture sweep
under `.external-fixtures/render-parity-samples-shared-body3/`; use
`just render-parity-samples-shared-body3-ash-gated` when Ash should join the
same gate under `.external-fixtures/render-parity-samples-shared-body3-ash-gated/`.
Both select `rgb-shared-nonblack-interior3px >= 28.5 dB`, comparing only pixels
where both renderers have non-black model content after dropping a three-pixel
local raster-ownership band. The current Ash-gated shared-body floor is the
constraint sample at wgpu `28.6412 dB` / Bevy `28.6300 dB` / Ash `28.6392 dB`,
while Seed-san improves to wgpu `31.9233 dB` / Bevy `31.2229 dB` / Ash
`31.9056 dB` under this stricter shared-content mask.
Use `just render-parity-constraint-shared-body3-diagnostics` when investigating
that floor: outline-off improves the constraint sample to wgpu `31.0592 dB` /
Bevy `31.0407 dB`, while normal-map-off stays flat at wgpu `28.6428 dB` /
Bevy `28.6321 dB`.
Use `just render-parity-constraint-shared-body3-pass-summary` after the
shared-body3 sweep to regenerate base/outline pass ownership counts for the
constraint top-64 hotspots; use
`just render-parity-constraint-shared-body3-pass-summary-ash-gated` after the
Ash-gated sweep for matching wgpu/Bevy/Ash summaries. The current Ash-gated
diagnostic shows all three Rust renderers share the same pass-level ownership
shape: `64/64` hotspots have a visible frontmost candidate, `62-63/64` are
within `0.25px` of an internal edge, and actual-vs-frontmost pass matches are
`51/64`. The same report includes frontmost-to-nearest surface transition
counts, currently showing that the largest constraint residuals are
base-material seams such as `EyeIris_00_EYE -> Hair_00_HAIR` and
`Hair_00_HAIR -> HairBack_00_HAIR` rather than an Ash-specific ownership path.
Use `just render-parity-texture-boundary-generated` for the focused source-like
base-texture UV-boundary guard. It generates
`.external-fixtures/generated/texture-boundary.vrm.gltf`, renders `base-color`
with outlines disabled, compares direct `.imqraw` buffers, and writes reports
under `.external-fixtures/render-parity-texture-boundary-generated/`. The
current guard has exact alpha parity and selected
`rgb-shared-nonblack-interior1px` PSNR wgpu `52.9232 dB` / Bevy `49.5132 dB`,
with max selected-channel deltas `7` / `6`.
Use `just render-parity-texture-selection-generated` for the focused
per-material base texture selection guard. It generates
`.external-fixtures/generated/texture-selection.vrm.gltf`, renders `base-color`
with outlines disabled, compares direct `.imqraw` buffers, and writes reports
under `.external-fixtures/render-parity-texture-selection-generated/`. The
current guard has exact alpha parity and selected
`rgb-shared-nonblack-interior1px` PSNR wgpu `58.0021 dB` / Bevy `52.5998 dB`,
with max selected-channel deltas `1` / `2`.
Use `just render-parity-split-ownership-generated` for the focused source-like
same-material split-mesh overlap guard. It generates
`.external-fixtures/generated/split-ownership.vrm.gltf` with meshes named
`wear_4` and `wear` sharing material `huku_bake`, renders `base-color` with
outlines disabled, compares direct `.imqraw` buffers, and writes reports under
`.external-fixtures/render-parity-split-ownership-generated/`. The current
guard has exact alpha parity, selected `rgb-shared-nonblack-interior1px` PSNR
wgpu `51.5385 dB` / Bevy `49.9613 dB`, and max selected-channel delta `7` for
both renderers. Use `just render-parity-split-ownership-owner-generated` to
render owner IDs for the same fixture; the expected current result is wgpu/Bevy
owner agreement with only a three-pixel owner tail against three-vrm.
Use `just render-parity-subpixel-ownership-generated` for the focused
source-like same-material subpixel ownership guard. It generates
`.external-fixtures/generated/subpixel-ownership.vrm.gltf` with material
`huku_bake`, near-subpixel triangle seams, and a high-contrast embedded base
texture, then renders `base-color` with outlines disabled. The current guard has
exact alpha parity and selected `rgb-shared-nonblack-interior1px` PSNR wgpu
`43.8282 dB` / Bevy `43.4195 dB`, with max selected-channel deltas `161` /
`160`. Its hotspot summaries keep `32/32` hotspots on the same base material
and `28/32` on the same frontmost triangle, so it is a regression guard for the
small generated version of the Seed-san `huku_bake` class rather than a full
reproduction of the real-model gradient floor.
Use `just render-parity-material-seam-generated` for the focused source-like
base-material seam guard. It generates
`.external-fixtures/generated/material-seam.vrm.gltf`, renders `base-factor`
with outlines disabled, compares direct `.imqraw` buffers, and writes hotspot
maps under `.external-fixtures/render-parity-material-seam-generated/`. The
current guard has exact alpha parity and selected
`rgb-shared-nonblack-interior1px` PSNR wgpu/Bevy `42.1555 dB`, with only `7`
changed shared-nonblack interior pixels concentrated on the generated diagonal
material seam.
Use
`--render-mtoon-time SECONDS` for MToon material-update parity checks such as UV
animation; `just render-parity-uv-animation` stores its time-advanced sample
under `.external-fixtures/render-parity-uv-animation/` so it does not overwrite
the canonical static sweep. Use `just render-parity-real-normal-maps` for the
focused real-fixture review of the known official tangentless normal-map
fixtures; it writes `.external-fixtures/render-parity-real-normal-maps/`. Use
`just render-parity-real-normal-maps-ash-gated` when Ash should join the same
selected-PSNR and exact-alpha gate.
Use `just render-parity-normal-maps-off` to disable normal maps across
three-vrm, wgpu, and Bevy when isolating whether a real-fixture delta comes from
tangentless normal-map behavior or another part of the material/geometry path.
Use `just render-parity-normal-maps-derivative` only as a diagnostic for the
shader-derivative tangent-frame fallback; the current measured path is worse
than generated tangents on Seed-san and is not the default guard.
Use `just render-parity-outline-off` as a diagnostic for separating MToon
outline expansion deltas from material, skinning, and pose deltas.
Use `just render-parity-seed-owner-id-outline-off-diagnostic`
to render Seed-san owner IDs with outlines disabled, matching the focused
base-color residual slice. Use
`just render-parity-seed-base-color-outline-off-owner-hotspots`
after `just render-parity-seed-base-color-outline-off-diagnostic` to project
the top shared-body hotspot pixels through the browser owner diagnostic. The
local runner's `--render-screen-jitter-x` and `--render-screen-jitter-y` options
accept negative values, so focused sweeps can test
`--render-screen-jitter-x -0.25` without using an equals-form workaround.
`tools/render-parity/summarize-render-hotspots.rs` turns
`map-render-hotspots.rs` JSON into compact JSON/Markdown review artifacts. The
outline-off Seed-san diagnostic writes
`.external-fixtures/render-parity-seed-base-color-outline-off-diagnostic/reports/Seed-san.{wgpu,bevy}-vs-three-vrm.hotspots.summary.{json,md}`;
the current summary keeps the focused blocker readable: `64/64` hotspots have
frontmost visible candidates, `58/64` are within `0.25px` of an edge, and Rust's
actual base-texture sample is closer to the CPU frontmost texture than the
three-vrm rendered expected color for `43/64` hotspots.
Use
`just render-parity-seed-base-color-flat32-outline-off-diagnostic`
when you need a PNG-free low-gradient interior cross-check for the same focused
Seed-san base-color slice. It writes direct `.imqraw` reports and delta reports
under
`.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/`
with selected metric `rgb-shared-nonblack-flat32-interior1px`. The domain keeps
shared non-black one-pixel interiors only when both expected and actual 3x3 RGB
neighborhoods stay within `32` channel values of the center pixel, so it helps
separate stable interior material/color parity from local texture/material/fill
edges. Current values are wgpu `49.7607 dB` and Bevy `47.5483 dB`, while the
ordinary shared-body outline-off score remains around `32.4 dB`; keep the
ordinary metric as the compatibility pressure and use flat32 as a diagnostic
classification signal. The same recipe also writes
`*.deltas.gradient.json` with domain
`shared-nonblack-gradient-interior1px`, the shared-body interior complement.
Current gradient-complement scores are wgpu `26.3277 dB` over `2638` pixels and
Bevy `26.3322 dB` over `2642` pixels, with max channel deltas `219` / `218`.
The recipe also maps those gradient deltas through
`map-render-hotspots.rs` and writes
`.hotspots.gradient.summary.{json,md}`. The current gradient hotspot summaries
match the outline-off shared-body shape: `64/64` frontmost candidates are
visible, `58/64` are within `0.25px` of a frontmost edge, every top hotspot is
base pass, and Rust actual is closer than three-vrm expected to the CPU
frontmost base texture for `44/64` hotspots. The compact summary also reports
same-triangle and nearest-edge-neighbor matches. Current gradient top-64
hotspots have only `6/64` same-triangle matches and `0/64` edge-neighbor
matches for both actual and expected, with large base-UV distances from the
frontmost candidate (wgpu actual/expected mean `0.6707` / `0.5730`, Bevy
`0.6752` / `0.5780`). The hotspot mapper now also reports CPU frontmost
base-texture local RGB gradient. The same top-64 gradient summaries show low
frontmost texture gradients, with mean/max wgpu `4.7577` / `25.4951` and Bevy
`4.6618` / `25.4951`, and `0/64` hotspots at `>=32`. Treat this as evidence
for real-model high-gradient UV island/material ownership rather than a simple
same-surface high-frequency texture sampling or adjacent-edge fill-rule issue.
Use
`just render-parity-seed-base-color-flat32-gradient-owner-hotspots`
to project the same gradient top-64 pixels through three-vrm's rendered
`owner-id` diagnostic. Current owner projection confirms that the browser
renders a non-zero owner for `64/64` gradient hotspots, but the rendered owner
matches the CPU center-sample frontmost candidate for only `27/64`. Subpixel
recovery finds `49/64`, all depth-rank `1`, with centers spread across
`0.25/0.5/0.75` offsets; one-pixel neighbor recovery finds `44/64` with mixed
offsets. The strongest rendered-owner material bucket is `huku_bake` (`24/64`),
matching the real-model UV/material-boundary interpretation.
Use
`just render-parity-seed-base-color-nearest-diagnostic` to
force nearest/no-mip texture sampling in three-vrm, wgpu, and Bevy for the same
Seed-san base-color outline-off slice. The current nearest run reports
`rgb-shared-nonblack-interior1px` wgpu `33.5501 dB` and Bevy `33.4815 dB`;
the gradient-complement diagnostic remains low at wgpu `27.1258 dB` and Bevy
`27.1208 dB` with max channel delta `220`. Treat this as evidence that the
remaining high-gradient floor is not explained by a broad mip/filter mismatch.
The canonical local runner now uses
`--render-background opaque-black`, so the three-vrm reference, wgpu capture,
and Bevy capture are all reviewed with the same opaque-background contract. Use
`--render-background transparent` only for explicit alpha-mask and silhouette
audits. Generated transparent-material guards currently include
`just render-parity-transparent-generated`,
`just render-parity-transparent-generated-ash-gated`,
`just render-parity-transparent-high-contrast`,
`just render-parity-transparent-high-contrast-ash-gated`,
`just render-parity-transparent-broad`,
`just render-parity-transparent-broad-ash-gated`,
`just render-parity-transparent-texture-transform`,
`just render-parity-transparent-texture-transform-ash-gated`,
`just render-parity-transparent-queue-matrix`, and
`just render-parity-transparent-queue-matrix-ash-gated`, and
`just render-parity-transparent-alpha-modes`; use
`just render-parity-transparent-alpha-modes-ash-gated` when Ash should join the
exact-alpha visual gate. Use `just render-parity-transparent-mask-texture` for
texture-alpha MASK cutoff and BLEND partial-alpha parity, or
`just render-parity-transparent-mask-texture-ash-gated` to include Ash; it
covers `baseColorTexture` alpha combined with baseColorFactor alpha, keeps
alpha buckets exact across three-vrm/wgpu/Bevy/Ash (`transparent=40720`,
`opaque=13552`, `partial=11264`), allows only 1-LSB alpha rounding, and
currently reports selected `rgb-visible` PSNR wgpu `55.3599 dB`, Bevy
`51.2036 dB`, and Ash `55.3599 dB` with max selected channel delta `<= 3`. Use
`just render-parity-transparent-lighted` for overlapping transparent layers
that also exercise MToon direct/shade/rim/emissive accumulation, and use the
`*-ash-gated` variant when Ash should participate. Use
`just render-parity-transparent-depth-stack` or
`just render-parity-transparent-depth-stack-ash-gated` for same-render-order
BLEND layers at different depths, including one texture-alpha layer; it keeps
alpha buckets exact while allowing only 1-LSB alpha channel rounding. The
alpha-modes guard specifically covers
OPAQUE alpha forcing, MASK `alphaCutoff` pass/fail behavior, and BLEND
`alphaCutoff` ignore behavior across three-vrm, wgpu, Bevy, and Ash. Generated
MToon material guards also include `just render-parity-mtoon-textures-generated`
for texture-slot parity and `just render-parity-mtoon-normal-generated` for the
current tangentless normal-map regression guard, which now enforces
`rgb-interior1px >= 46.5 dB`.
The local runner writes the three-vrm, wgpu, and Bevy PNGs from
their RGBA artifacts through the same Rust PNG encoder, so review images match
the exact buffers compared by the RGBA JSON diagnostic path. It decodes each
PNG after writing and requires a byte-for-byte match with the corresponding
RGBA artifact, including alpha. Before that, it verifies that each renderer's
direct `.imqraw` artifact matches the same RGBA JSON bytes, so the numeric gate
and visual-review artifacts cannot silently diverge. It also checks that the
wgpu and Bevy alpha masks stay within
`--render-alpha-mismatch-tolerance` pixels of the three-vrm reference. The
render-parity run recreates its managed output directories first, so stale
direct-capture smoke PNGs are not mixed into the canonical review set. The
summary table lives at `.external-fixtures/render-parity/summary.md` and is also
embedded at the top of `visual-review.html`; use it as the first stop for
selected PSNR, max channel delta, alpha mismatch, and pass/fail status.
`review-manifest.json` links that same gate data to source fixtures,
reference/capture RGBA JSON, direct imqraw, PNG, diagnostic reports, and diff
heatmaps for downstream audits, and `tools/render-parity/validate-review-manifest.rs`
checks those links, comparison pass flags, report-local `expected`/`actual`
paths, and summary fields mirrored from the direct-imqraw numeric report. The
compared images live under `.external-fixtures/render-parity/three-vrm/`,
`.external-fixtures/render-parity/wgpu/`, and
`.external-fixtures/render-parity/bevy/`.
The runner keeps the reusable comparison logic Sans I/O where practical:
RGBA JSON parsing, alpha counting, diff heatmap pixel generation, PSNR report
summary extraction, and summary Markdown construction work on in-memory values,
while filesystem reads/writes remain in the surrounding runner functions.
Texture upload preparation follows the same direction: `vrm-io` exposes
`image_bytes_to_rgba8` / `image_data_to_rgba8` for source image byte
normalization and `generate_rgba_mip_chain` for validated mip planning. These
helpers take dimensions, formats, and bytes, then return renderer-neutral RGBA8
bytes or mip levels without touching files or GPU APIs. The wgpu and Bevy
captures both use those helpers before converting the plan into backend texture
uploads, so ash/custom-engine paths can reuse the same validated texture input
data. CPU-side texture diagnostics also use `vrm-io::CpuRgba8Image`, which
samples RGBA8 channels with repeat/linear filtering and an explicit
`Rgba8SamplingOrigin`; this keeps outline-width texture sampling comparable
between the concrete wgpu/Bevy captures and future ash/custom renderer tests.
Material texture slot selection is shared through
`LoadedVrm::material_texture_slots`, including MToon texture slots, glTF
base/normal fallbacks, emissive and occlusion textures, and outline/UV-animation
mask slots after texture-index validation.
The matching `GltfMaterialTextureSlots::binding_plan` keeps shader-slot
texture index, sRGB/linear table selection, and white/black/neutral-normal
fallback policy in renderer-neutral data before wgpu, Bevy, ash, or custom
engines allocate concrete texture handles.
Material UV transform selection is shared through
`LoadedVrm::material_uv_transforms`, including MToon transforms, glTF
base/normal/emissive/occlusion fallbacks, shade fallback-to-base behavior, and
MToon UV animation scroll/rotation at a requested time.
Shader-facing UV uniform packing is shared through
`GltfMaterialUvTransforms::uniform_plan`, covering offset/scale defaults,
rotation slots, UV animation scroll/rotation, and the current texCoord0-only
policy before concrete captures convert the plan into backend uniform types.
Material shading input selection is shared through
`LoadedVrm::material_shading_plan`, covering MToon versus glTF/PBR fallback
values, effective emissive strength, normal scale, rim/matcap parameters,
metallic/roughness, unlit state, and VRM0 compatibility flags before concrete
captures apply runtime expression color overrides.
Expression render effect selection is shared through
`LoadedVrm::expression_render_effects` and `GltfExpressionRenderEffects`,
covering binary expression weights, morph-target clears and accumulation,
material color binds, and texture-transform binds before concrete captures
convert them into mesh weights, material colors, and UV transform uniforms.
Primitive morph target evaluation is shared through
`GltfPrimitiveData::morphed_vertex`, covering local position, normal, and
tangent deltas before concrete captures apply skinning, outline expansion,
normal-map tangent generation, or backend mesh construction.
`skin_vertex_applies_weighted_joint_matrices_to_positions_and_normals` covers
non-identity weighted skinning so renderer captures and VRMA/posed parity do not
silently fall back to unskinned local vertex data.
CPU-side texCoord0 transform application is shared through
`vrm-io::transform_tex_coord_0`, covering offset/scale/rotation and the current
policy of ignoring transforms that target non-zero UV sets.
MToon capture lighting policy is shared through
`vrm-adapter::MtoonLightingConfig`, so the tuned diagnostic accumulator and the
reference-shaped `three-vrm` accumulator produce the same effective uniform
values for wgpu, Bevy, and future renderer examples.
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
audited against the actual reference scene conditions. The Rust capture paths
consume the glTF sampler min/mag/wrap policy extracted by `vrm-io`, including
whether a texture should use mip levels at all, and the wgpu capture binds
samplers per material texture slot rather than reusing the base texture sampler
for every slot. Mip chains are CPU-generated with CatmullRom downsampling for
the current capture parity path, and renderer-facing glTF material extraction
also exposes `KHR_materials_unlit` for non-MToon/PBR fallback materials. VRMC
MToon materials intentionally stay on the MToon shader branch even when the
glTF unlit extension is present, matching the measured three-vrm behavior of
the official UV-animation fixture. This keeps model-specific sampler and
material policy from being flattened into one renderer-global or
material-global rule. The generated MToon
light/color fixture now contains 13 swatches, including mid-ramp interpolation
cases and an MToon material that also carries `KHR_materials_unlit`, and the
swatch comparator is run after aggregate PSNR to catch per-term drift. Use
`just render-parity-mtoon-light-ash-generated` when the same
source-like light fixture should also emit Ash readback artifacts and Ash
swatch reports; the Ash comparison remains non-gating in the review manifest,
but the recipe verifies aggregate `rgb-interior1px` and named swatch color
parity for the Vulkan handoff path. For direct-light isolation,
`just render-parity-mtoon-light-direct-generated` disables ambient
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
- Render parity is not yet fully satisfied across Rust renderers. P3 now has a PSNR comparator, RGBA artifact format, concrete three-vrm browser reference capture, textured wgpu offscreen capture, headless Bevy capture, glTF sampler-policy parity including wgpu per-slot sampler bindings, UV-animation fixture coverage, mask-material fixture coverage, generated transparent-material guards with Ash-gated base, high-contrast, broad, texture-transform, queue-matrix, alpha-mode, mask-texture, depth-stack, and lighted BLEND cases, generated tangentless normal-map parity, direct/ambient isolated MToon light-color guards, Ash mip-aware Vulkan texture materialization and mask-alpha parity, a six-fixture real sweep gated at selected `rgb-visible >= 34 dB` for wgpu/Bevy/Ash, a transparent-background six-fixture real sweep gated at `rgb-all >= 32 dB` for wgpu/Bevy/Ash, a focused real tangentless normal-map sweep gated at `rgb-visible >= 34 dB` for wgpu/Bevy/Ash, matching object-body `rgb-nonblack-interior1px >= 27.4 dB` and stricter shared-body `rgb-shared-nonblack-interior3px >= 28.5 dB` diagnostic sweeps for wgpu/Bevy/Ash, and a VRM1/current-official subset gated at `34 dB`; broader real-model PSNR, real tangentless normal-map fixture breadth beyond the current pair, model-body parity above the current floors, broader real transparent fixture breadth, and higher final thresholds are still pending.

## Current Coverage Snapshot

Measured locally on 2026-06-25 with:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

| Scope | Region coverage | Line coverage |
| --- | ---: | ---: |
| Workspace total | 80.97% | 82.96% |
| `vrm-adapter-ash` | 83.64% | 85.42% |
| `vrm-adapter-bevy` | 76.02% | 77.53% |
| `vrm-adapter-wgpu` | 29.21% | 30.75% |
| `vrm-adapter` | 74.67% | 81.01% |
| `vrm-core` | 75.32% | 79.53% |
| `vrm-diagnostics` | 87.50% | 90.60% |
| `vrm-io` | 88.34% | 87.32% |
| `vrm-osc` | 68.00% | 70.53% |
| `vrm-protocol` | 93.02% | 91.16% |
| `vrm-runtime` | 93.17% | 93.92% |
| `vrm-sans-io` | 93.52% | 96.55% |
| `vrm-vmc` | 77.11% | 77.76% |
| `facade src/lib.rs` | 93.26% | 96.78% |

The current external fixture tests cover recursive fixture discovery, semantic
IO loading including the Alicia VRM0 compatibility sample, adapter spring rest
capture/stepping, Seed-san center-space spring golden comparison, Alicia VRM0
spring golden comparison, Alicia VRM0 humanoid rest/writeback golden comparison,
collider-heavy spring directory comparison, fixture-driven node constraint
manager ordering/writeback comparison, Seed-san raw/normalized humanoid
rest-state and posed writeback comparison, baseline plus dense Seed-san
`test.vrma` application comparison, and branch-only `idle_loop.vrma`
application comparison on real VRM/VRMA files without committing those binaries.
Generated VRMA diagnostics cover stable warnings for ignored non-hips humanoid
translation tracks, hips translation rest-height scaling, stable errors for
invalid expression/lookAt animation paths, and normal-gate application of
humanoid, preset/custom expression, and lookAt tracks through the adapter
writeback path. Renderer-facing generated glTF coverage now also includes
primitive `COLOR_0`, morph target deltas, mesh/node default morph weights,
public MToon renderer material plans, public primitive pipeline plans, glTF
sampler min/mag/wrap extraction, same-image multi-texture sampler preservation,
`KHR_materials_unlit` extraction for PBR fallback material data, renderer-neutral
RGBA8 image normalization, renderer-neutral RGBA channel sampling,
renderer-neutral material texture slot resolution and binding plans,
renderer-neutral material UV transform resolution, renderer-neutral UV uniform
packing, renderer-neutral material shading input selection, renderer-neutral
expression render effect planning, renderer-neutral material pipeline planning,
renderer-neutral joint matrix assembly and CPU skinning, renderer-neutral
texCoord0 transform application, renderer-neutral MToon lighting accumulator
resolution, renderer-neutral outline expansion, renderer-neutral texture image
lookup/RGBA8 conversion, renderer-neutral texCoord0 fallback, and
renderer-neutral prepared vertex generation before renderer-specific buffer
creation, renderer-neutral outline-width texture lookup, renderer-neutral
whole-primitive UV fallback generation, and renderer-neutral RGBA mip-chain
generation. Normal-map fallback policy is shared through
`GltfNormalMapPlan`, so authored tangents, generated tangents, derivative
normal fallback, and normal-scale delivery are decided before renderer-specific
buffer/material construction. MToon/PBR shader flag and parameter packing is
shared through `GltfMaterialRenderExtraPlan`, so V0 compatibility, PBR fallback,
three-vrm light accumulation, derivative normal fallback, unlit state, metallic,
roughness, occlusion strength, and direct-light scale are also decided before
renderer-specific material construction. Facade-level headless scene
construction and zero-delta world-matrix evaluation are shared before
renderer-specific buffer creation. Morph/skin/world-transformed primitive
vertices are shared through `GltfPrimitiveData::transformed_vertices`, so
renderer-specific paths receive the same prepared position, normal, tangent,
UV, and color inputs before constructing wgpu buffers or Bevy meshes. Outline
width texture sampling, outline UV transform application, sampling origin, and
expanded outline positions are shared through
`GltfPrimitiveData::outline_vertices` before renderer-specific outline buffers
or Bevy meshes are built.
Expression-applied renderer material state is also shared through
`LoadedVrm::expression_material_shading_plan` and
`LoadedVrm::expression_material_uv_transforms`, while MToon outline material
state is shared through `LoadedVrm::expression_mtoon_outline_plan`.

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
15. Ash/Vulkan materialization coverage now includes `cargo run --example ash_mtoon_pipeline_materialization`, which maps the same public MToon plans into Vulkan-shaped descriptor-set layouts, separate sampled-image/sampler writes, push constants, rasterization/depth/blend keys, WGSL-derived SPIR-V shader names, and sorted base/outline draw queues. The `vrm-adapter-ash` crate also tests `ash_renderer_frame_from_plan` producing per-pipeline `AshGraphicsPipelinePlan` values with vertex layout and color/depth formats for the real ash offscreen drawable example.
16. Bevy hierarchy readback now covers real `ChildOf` ECS hierarchy components, deriving `BevyRuntimeSceneState` parent/child links before spring parity and runtime-driver ticks.
17. Bevy MToon materialization coverage now includes `cargo run --example bevy_mtoon_materialization`, which maps MToon pass plans and runtime material state into a Bevy-facing asset without shader policy.

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
