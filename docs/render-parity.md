# Render Parity

P3 render parity compares three rendering paths:

- three-vrm reference render.
- Bevy adapter render.
- wgpu/custom-engine render.

Rendered artifacts remain under `.external-fixtures/render-parity/` and are not
committed. Each renderer should export a source-like RGBA JSON artifact:

```json
{
  "width": 2,
  "height": 1,
  "rgba": [255, 0, 0, 255, 0, 0, 255, 255]
}
```

The RGBA artifact is intentionally simple so browser canvas, Bevy readback,
wgpu readback, or an ash staging image can all write the same format before a
PNG visual artifact is added for human review.

## PSNR

Use the dependency-free comparator:

```powershell
node tools\render-parity\compare-psnr.mjs `
  --expected .external-fixtures\render-parity\three-vrm\Seed-san.frame000.rgba.json `
  --actual .external-fixtures\render-parity\wgpu\Seed-san.frame000.rgba.json `
  --out .external-fixtures\render-parity\reports\Seed-san.wgpu.frame000.psnr.json `
  --metric rgb-visible `
  --fail-under 40
```

The report contains dimensions, MSE, PSNR, maximum channel delta, maximum pixel
delta, alpha counts/mismatches, RGB-only opaque/visible/interior metrics, the
selected metric, and pass/fail status. Exact matches report `"Infinity"` for
PSNR. The comparator accepts `--metric rgba`, `--metric rgb-opaque`,
`--metric rgb-visible`, `--metric rgb-nonblack`,
`--metric rgb-interior1px`, `--metric rgb-visible-interior1px`, and
`--metric rgb-nonblack-interior1px`; pass/fail thresholds use the selected
metric. The nonblack metrics are intended for opaque-black review sweeps where
empty background pixels should not dilute the model-body color error. It also
accepts
`--max-selected-channel-delta` and `--max-alpha-delta` for fixture-specific
worst-case guards, and the Rust local runner forwards them as
`--render-max-selected-channel-delta` and `--render-max-alpha-delta`. The local
render-parity runner defaults to `rgb-visible`; with the canonical opaque-black
review background this is the visible RGB surface metric, and it also remains
useful for explicit transparent alpha-mask audits. Use `rgb-nonblack` for
opaque-black whole-model diagnostics and `rgb-nonblack-interior1px` when
one-pixel silhouette/raster edges should be dropped from that diagnostic. Use
`rgb-visible-interior1px` when transparent interiors must be measured while
still dropping one-pixel silhouette edges.

For the full local Seed-san parity loop, use the Rust local CI runner:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- `
  --render-parity `
  --skip-core `
  --skip-coverage `
  --skip-download `
  --skip-three-vrm-build `
  --skip-playwright-install `
  --three-vrm-root D:\git\three-vrm `
  --render-background opaque-black `
  --render-mtoon-light-accumulation three-vrm `
  --render-fixture Seed-san.vrm `
  --render-fixture VRM1_Constraint_Twist_Sample.vrm `
  --render-fixture .external-fixtures\official\vrm-specification\samples\VRMC_materials_mtoon_UV_Animation_Test\VRMC_materials_mtoon_UV_Animation_Test.vrm `
  --render-fixture .external-fixtures\official\vrm-specification\samples\VRMC_vrm_expressions_isBinary_Overridden\VRMC_vrm_expressions_isBinary_Overridden.vrm `
  --render-fixture .external-fixtures\official\vrm-specification\samples\VRMC_vrm_expressions_isBinary_Overrides\VRMC_vrm_expressions_isBinary_Overrides.vrm `
  --render-fixture .external-fixtures\official\UniVRM\AliciaSolid_vrm-0.51.vrm
```

Without the `--skip-*` flags, the same script can download external fixtures,
prepare Playwright, and build the pinned three-vrm checkout under
`.external-fixtures/three-vrm`. If no `--render-fixture` is passed, the render
set defaults to `Seed-san.vrm`. The render pass writes per-fixture artifacts:

- `.external-fixtures/render-parity/three-vrm/<fixture>.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/wgpu/<fixture>.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/bevy/<fixture>.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/reports/<fixture>.{wgpu,bevy}-vs-three-vrm.psnr.json`
- `.external-fixtures/render-parity/diff/<fixture>.{wgpu,bevy}-vs-three-vrm.diff.png`
- `.external-fixtures/render-parity/summary.md`
- `.external-fixtures/render-parity/visual-review.html`

`summary.md` is the compact audit artifact for PSNR and alpha review. It lists
the selected metric, background, MToon light accumulation mode, per-fixture
wgpu/Bevy selected PSNR, max selected-channel delta, alpha mismatch count,
alpha max delta, and pass/fail status. `visual-review.html` embeds the same
summary before the side-by-side PNGs and diff heatmaps.

Open `visual-review.html` locally to compare the three PNGs side-by-side with
their PSNR reports and diff heatmaps. In the heatmaps, red shows RGB-channel
delta and blue shows alpha-channel delta, amplified for review. It is generated
data and stays outside git.

The local runner encodes every renderer PNG from its `.rgba.json` artifact with
the same Rust PNG writer. The canonical visual-review run uses
`--render-background opaque-black`, so the three-vrm reference PNG, wgpu PNG,
and Bevy PNG all use the same fully opaque background/alpha contract.
`--render-background transparent` remains available for targeted alpha-mask
audits when the goal is to inspect the silhouette or transparent-background
readback path. This keeps preview PNGs aligned with the exact RGBA buffers used
for PSNR instead of relying on browser element screenshots or canvas
compositing. At the start of
each render-parity run, the managed `three-vrm`, `wgpu`, `bevy`, `reports`, and
`diff` directories are recreated so older direct-capture smoke images cannot be
mistaken for the current canonical comparison set. Each PNG is decoded after
writing and must match its RGBA artifact bytes, including alpha.

The official sample sweep intentionally includes the expression override
fixtures because their text and meter materials exercise glTF/MToon `MASK`
alpha. The wgpu and Bevy capture shaders treat pixels that survive the cutoff
as fully opaque, matching three-vrm/three.js alpha-test output rather than
preserving the source texture alpha in the readback buffer. It also includes
the external UniVRM Alicia VRM0 fixture, kept outside git, to keep legacy
transparent and transparent-Z-write materials in the same rendered-output
parity loop.

For a transparent-background sweep over the real external sample set, run:

```powershell
just render-parity-real-transparent
```

This writes `.external-fixtures/render-parity-real-transparent/` and uses
`rgb-all` with `--render-fail-under 32`, while alpha-mask drift is enforced
separately. The current run keeps alpha mismatches under `64` for all six
fixtures: Seed-san wgpu/Bevy `25/32`, constraint `11/11`, UV animation `0/0`,
expression mask samples `12/12`, and Alicia VRM0 `32/32`. Selected `rgb-all`
PSNR is Seed-san wgpu `34.5647 dB`, Seed-san Bevy `34.0434 dB`, constraint
wgpu `36.2028 dB`, constraint Bevy `36.1877 dB`, UV animation wgpu
`35.2517 dB`, UV animation Bevy `35.2073 dB`, expression mask samples around
`39.29-39.30 dB`, Alicia wgpu `32.6863 dB`, and Alicia Bevy `32.6757 dB`.
Review
`.external-fixtures/render-parity-real-transparent/visual-review.html` when
working on broader transparent silhouettes, VRM0 material ordering, or runtime
pose/material breadth.

For a focused real-fixture sweep over currently known MToon normal-map fixtures
whose primitives omit glTF `TANGENT`, run:

```powershell
just render-parity-real-normal-maps
```

This writes `.external-fixtures/render-parity-real-normal-maps/` and currently
covers the unique official/local fixtures reported by `inspect-mtoon-fixtures`:
`Seed-san.vrm` has `6` normal-mapped primitives without tangents, and
`VRM1_Constraint_Twist_Sample.vrm` has `3`. The run uses the canonical
opaque-black review background, selected `rgb-visible`, exact alpha parity, and
`--render-fail-under 34`. Current selected PSNR is Seed-san wgpu `34.5647 dB` /
Bevy `34.0434 dB`, and constraint sample wgpu `36.2028 dB` / Bevy
`36.1877 dB`, all with alpha mismatches `0`. Review
`.external-fixtures/render-parity-real-normal-maps/visual-review.html` and the
diff heatmaps when changing tangent generation, normal sampling, back-face TBN
handling, or MToon light/color accumulation.

For a normal-map-off diagnostic over the same real fixtures, run:

```powershell
just render-parity-normal-maps-off
```

This writes `.external-fixtures/render-parity-normal-maps-off/` and disables
normal maps in the three-vrm reference, wgpu capture, and Bevy capture. It is a
diagnostic for separating tangentless normal-map residuals from material,
skinning, pose, and outline deltas. The current run uses selected
`rgb-visible`, exact alpha parity, and `--render-fail-under 35`. Seed-san
improves from the normal-map-enabled sweep to wgpu `36.0858 dB` / Bevy
`35.3848 dB`, while the constraint sample remains roughly flat at wgpu
`36.2203 dB` / Bevy `36.2054 dB`. That makes Seed-san's remaining residual
partly normal-map/tangentless-related, while the constraint sample continues to
point more toward outline, geometry, or rasterization differences.

For a shader-derivative tangent-frame diagnostic over the same real fixtures,
run:

```powershell
just render-parity-normal-maps-derivative
```

This forwards `--render-normal-map-mode derivative` into the wgpu and Bevy
captures while leaving three-vrm on its native tangentless fallback. It is not
the default renderer path: the 2026-05-30 measurement passed exact alpha parity
but was worse than the generated-tangent path, with Seed-san wgpu `30.8717 dB`
/ Bevy `30.6436 dB` and constraint wgpu `35.9214 dB` / Bevy `35.9149 dB`.
Keep using generated tangents for the current real-fixture guard until a closer
formulation beats `just render-parity-real-normal-maps`.

The Rust capture paths default to `--mtoon-light-accumulation three-vrm`. That
mode uses the closer WebGL MToon accumulator shape for light/color audits:
direct diffuse is normalized by the `DirectionalLight(Math.PI)` setup, indirect
diffuse uses `pbrAmbient`, rim lighting uses the same direct-plus-indirect
accumulator (`1.0 + pbrAmbient`) as the three-vrm non-physical-light path, and
the final exposure multiplier is fixed at `1.0` instead of inheriting the old
PSNR-tuned `0.78` capture coefficient. The older `tuned` mode remains available
as an explicit diagnostic switch. Re-run the focused exact-light path with:

```powershell
just render-parity-light-three-vrm
```

The current focused exact-accumulator run reports Seed-san wgpu `34.5645 dB` /
Bevy `34.0434 dB` and constraint sample wgpu `36.2028 dB` / Bevy
`36.1877 dB` on selected `rgb-visible`, with exact alpha parity.

For an outline-off diagnostic that disables MToon outline expansion in the
three-vrm reference and both Rust captures, run:

```powershell
just render-parity-outline-off
```

This writes `.external-fixtures/render-parity-outline-off/` and is meant to
separate outline expansion from material, skinning, and pose deltas. The current
diagnostic keeps exact alpha parity. Seed-san stays essentially flat at wgpu
`34.4260 dB` / Bevy `34.4200 dB`, so its remaining error is not explained by
outline alone. The constraint sample improves from the normal sweep's
wgpu `36.2028 dB` / Bevy `36.1877 dB` to wgpu `37.1221 dB` / Bevy
`37.0988 dB`, which marks outline expansion as a real contributor for that
fixture. The capture outline geometry now follows three-vrm's local/object
normal expansion with normal-matrix length compensation after morph/skin
position deformation, and wgpu also applies the three-vrm outline clip-depth
nudge. Re-running the focused guards did not move the measured PSNR, so
remaining constraint outline residuals are more likely in rasterization/fill,
material color, or Bevy's lack of a custom outline vertex stage than in
transform-scale or pre/post-skin offset ordering.
The concrete wgpu and Bevy captures also accept `--outline-width-scale`, and
the local runner forwards it through `--render-outline-width-scale` for
diagnostic sweeps. The default scale `1.0` remains the measured best setting on
the constraint sample; a corrected `0.9` run dropped to wgpu `35.4088 dB` and
Bevy `35.3977 dB`, while `1.0` stayed at wgpu `35.9456 dB` and Bevy
`35.9394 dB` under the previous tuned sweep.

For isolated experiments, the local runner can also override the three-vrm
reference light setup with `--render-three-vrm-directional-intensity`,
`--render-three-vrm-directional-{x,y,z}`, and
`--render-three-vrm-ambient-intensity`. The Rust capture examples still use
their own `--render-pbr-ambient` / MToon accumulation flags, so direct-only or
ambient-only experiments must set both sides explicitly.

For a generated MToon light/color accumulation audit that isolates shader
terms without relying on redistributable binary model assets, run:

```powershell
just render-parity-mtoon-light-generated
```

This writes `.external-fixtures/generated/mtoon-light.vrm.gltf` and renders it
into `.external-fixtures/render-parity-mtoon-light-generated/`. The fixture
contains twelve opaque MToon quads for forced base lighting, forced shade
lighting, ambient behavior with a glTF `occlusionTexture` that three-vrm MToon
ignores, parametric rim, matcap rim, mixed rim/matcap, two low-toony ramp
materials with angled normals, two mid-ramp interpolation cases, pure
`emissiveFactor`, and `emissiveTexture` multiplied by
`KHR_materials_emissive_strength`. It
uses `--mtoon-light-accumulation three-vrm`,
`--render-background transparent`, and selected metric `rgb-interior1px` so a
one-pixel silhouette/rasterization disagreement does not dominate the shader
color score. The current run reports selected PSNR wgpu `59.7573 dB` with max
selected channel delta `2`, and Bevy `51.1653 dB` with max selected channel
delta `2`; the aggregate recipe now fails if selected channel delta exceeds
`2`. The browser reference logs that `aoMap`/`aoMapIntensity` are not
ShaderMaterial properties for WebGL MToon, so Rust keeps MToon occlusion
disabled for parity while still extracting and applying glTF occlusion to
non-MToon/PBR fallback materials. The expected edge-only alpha mismatch is
`330` pixels and the recipe allows up to `512`; use the strict transparent
generated fixture below for alpha/blend correctness. Review
`.external-fixtures/render-parity-mtoon-light-generated/visual-review.html`
when changing MToon accumulation code. The remaining broad-fixture PSNR gap is
therefore more likely to come from real model runtime/material breadth,
post-correction, outline edges, and fixture coverage than from this isolated
base/shade/rim/matcap/emission color formula.

The recipe also runs the Rust swatch comparator after aggregate PSNR. It labels
each opaque generated swatch by connected alpha component, drops one-pixel
edges, and writes per-swatch color reports at
`.external-fixtures/render-parity-mtoon-light-generated/reports/mtoon-light_vrm.wgpu.swatches.json`
and
`.external-fixtures/render-parity-mtoon-light-generated/reports/mtoon-light_vrm.bevy.swatches.json`.
The current swatch guard requires wgpu max channel delta `<= 2` with every
swatch at least `50 dB`; Bevy is allowed max channel delta `<= 2` with every
swatch at least `47 dB`. This prevents a high aggregate score from hiding a
broken individual MToon term such as forced shade, rim, matcap, or emissive
strength. The same swatch guard is also run for the direct-only,
colored-direct, and ambient-only recipes below, so default, direct, colored
direct/rim, and indirect accumulation drift are separated in their own report
directories.

For the same generated fixture with ambient disabled on both sides, run:

```powershell
just render-parity-mtoon-light-direct-generated
```

This writes `.external-fixtures/render-parity-mtoon-light-direct-generated/`.
It sets the three-vrm reference ambient intensity to `0` and Rust
`pbrAmbient` to `0`, leaving only the `DirectionalLight(Math.PI)` path plus
rim/emission behavior. The current selected `rgb-interior1px` PSNR is wgpu
`59.7409 dB` and Bevy `51.6508 dB`, with max selected channel deltas `2` and
`2`. The recipe also writes direct-only swatch reports under
`.external-fixtures/render-parity-mtoon-light-direct-generated/reports/` and
enforces the same wgpu `<= 2` / Bevy `<= 2` per-swatch max channel deltas. This
keeps direct-light color parity measured separately from the default reference
run that includes ambient `0.1`.

For the same direct-only setup under a non-white directional light, run:

```powershell
just render-parity-mtoon-light-colored-generated
```

This writes
`.external-fixtures/render-parity-mtoon-light-colored-generated/`. It passes
`--render-directional-r 1.0 --render-directional-g 0.55
--render-directional-b 0.25` into the three-vrm reference, wgpu capture, and
Bevy capture while keeping ambient disabled. The current selected
`rgb-interior1px` PSNR is wgpu `59.5993 dB` and Bevy `51.9037 dB`, with max
selected channel deltas `2` and `2`. The per-swatch guard also passes, with
wgpu max channel delta `<= 2` and Bevy max channel delta `<= 2`, proving that
the direct diffuse and rim-light mix paths are not only matching for white
lights.

For the same generated fixture with directional light disabled on both sides,
run:

```powershell
just render-parity-mtoon-light-ambient-generated
```

This writes `.external-fixtures/render-parity-mtoon-light-ambient-generated/`.
It sets the three-vrm reference `DirectionalLight` intensity to `0` and Rust
`--render-direct-light-scale` to `0`, leaving the ambient `0.1` path plus
emission. The current selected `rgb-interior1px` PSNR is wgpu `61.7386 dB` and
Bevy `54.5071 dB`, with max selected channel deltas `2` and `2`. The recipe
also writes ambient-only swatch reports under
`.external-fixtures/render-parity-mtoon-light-ambient-generated/reports/` and
enforces the same per-swatch color bounds. This keeps ambient/indirect MToon
accumulation measurable separately from the direct-only and default light
setups.

For a generated MToon post-correction audit, run:

```powershell
just render-parity-mtoon-post-correction-generated
```

This writes `.external-fixtures/generated/mtoon-post-correction.vrm.gltf` and
renders it into
`.external-fixtures/render-parity-mtoon-post-correction-generated/`. The
fixture disables direct and ambient light and uses four emissive MToon
swatches: a mid-tone linear-to-sRGB case, two overbright emissive clamp cases,
and a transparent overbright case. The recipe selects
`rgb-visible-interior1px`, so opaque and partial-alpha interiors are measured
while the known one-pixel raster edge is excluded. Current selected PSNR is
wgpu `52.8818 dB` and Bevy `54.1650 dB`, both with max selected channel delta
`1`. Alpha buckets match in the partial region; after allowing one LSB for
partial alpha, only `144` edge pixels differ, under the recipe tolerance
`256`.

For a generated MToon texture-slot audit, run:

```powershell
just render-parity-mtoon-textures-generated
```

This writes `.external-fixtures/generated/mtoon-texture-slots.vrm.gltf` and
renders it into `.external-fixtures/render-parity-mtoon-textures-generated/`.
The fixture exercises `shadeMultiplyTexture`, `shadingShiftTexture`,
`rimMultiplyTexture`, `uvAnimationMaskTexture` at `mtoon-time=1.0`, and
`outlineWidthMultiplyTexture` while keeping all source data generated in the
repository. The current run has exact alpha buckets and mismatches `0`.
Selected `rgb-interior1px` PSNR is wgpu `53.8749 dB` and Bevy `51.2935 dB`,
with max selected channel delta `8`; the recipe enforces a `50 dB` floor,
selected-channel delta `<= 8`, and exact alpha.

For a generated MToon normal-map audit, run:

```powershell
just render-parity-mtoon-normal-generated
```

This writes `.external-fixtures/generated/mtoon-normal-texture.vrm.gltf` and
renders it into `.external-fixtures/render-parity-mtoon-normal-generated/`.
The fixture uses the texture-slot guard plus an additional normal-textured
MToon swatch. That swatch intentionally omits glTF `TANGENT`: adding a
synthetic tangent attribute triggered a three-vrm WebGL `vTangent` varying
validation failure, while the tangentless path exercises three-vrm's normal-map
fallback behavior. The current run has exact alpha buckets and mismatches `0`;
selected `rgb-interior1px` PSNR is wgpu `47.1013 dB` with max selected channel
delta `12`, and Bevy `46.0637 dB` with max selected channel delta `13`. The
Bevy path now keeps generated tangent frames when a primitive accessor includes
unreferenced vertices and enables `VERTEX_TANGENTS` for the custom material
shader. The recipe now enforces `rgb-interior1px >= 45 dB`. Treat this as the
current normal-map regression guard, not final visual parity; the remaining work
is to confirm real tangentless official primitives with higher thresholds.

For a stricter MToon light/color accumulation audit with non-default three-vrm
light units, run:

```powershell
just render-parity-mtoon-light-scaled-colored-generated
```

This reuses `.external-fixtures/generated/mtoon-light.vrm.gltf`, but captures it
with `DirectionalLight.intensity = 2.3561945`, `AmbientLight.intensity = 0.25`,
and a non-white directional color `(0.35, 0.72, 1.0)`. The local runner passes
`--render-sync-three-vrm-light-units`, so wgpu and Bevy receive
`direct-light-scale = directionalIntensity / PI` and
`pbr-ambient = ambientIntensity / PI`. This mirrors three-vrm's MToon shader,
where Lambert diffuse and rim's `directSpecular` use light units after division
by `PI`. Current selected `rgb-interior1px` PSNR is wgpu `61.9831 dB` and Bevy
`51.6960 dB`; max selected channel delta is `2` and `2` respectively. The
per-swatch report also passes with wgpu max channel delta `<= 2` and Bevy
`<= 2`, so direct diffuse, forced shade, ambient, rim/matcap, toon-ramp, and
emissive swatches are checked under scaled colored lighting.

For a generated expression morph audit, run:

```powershell
just render-parity-morph-expression-generated
```

This writes `.external-fixtures/generated/morph-expression.vrm.gltf` and
renders it into `.external-fixtures/render-parity-morph-expression-generated/`.
The fixture exposes a VRM1 `happy` expression with a morph target bind, a
material `color` bind, and a material texture-transform bind over an embedded
bufferView PNG. The render harness passes `--render-expression happy=1.0`
through three-vrm, wgpu, and Bevy. The current run has the expected tiny
fill-rule alpha mismatch (`3`, tolerance `8`), while selected
`rgb-interior1px` PSNR is wgpu `58.2703 dB` with max selected channel delta
`1`, and Bevy `50.8123 dB` with max selected channel delta `2`. This guards
expression morph, material-color, texture-transform, and binary-weight
semantics in the concrete render paths without committing binary assets.

For a license-safe transparent material audit, generate the local source-like
fixture and render it on a transparent background:

```powershell
just render-parity-transparent-generated
```

This writes `.external-fixtures/generated/transparent-blend.vrm.gltf` and
renders it into `.external-fixtures/render-parity-transparent-generated/`. The
fixture contains two overlapping VRM1 MToon `BLEND` primitives, one with
`transparentWithZWrite`, and one layer carries an embedded bufferView PNG
base-color texture, so the run checks partial-alpha accumulation, texture color
sampling, and depth-write policy without committing binary sample assets. The
alpha mismatch tolerance is intentionally zero for this generated fixture, and
the recipe now fails if selected `rgb-visible` falls below `49 dB`, selected RGB
channel delta exceeds `2`, or alpha delta is non-zero.
For a stronger transparent layer-ordering audit, use the high-contrast palette:

```powershell
just render-parity-transparent-high-contrast
```

This writes `.external-fixtures/generated/transparent-high-contrast.vrm.gltf`
and renders into
`.external-fixtures/render-parity-transparent-high-contrast/`. The output
includes side-by-side PNGs at `three-vrm/`, `wgpu/`, and `bevy/`, amplified
diff PNGs at `diff/`, PSNR JSON reports at `reports/`, and the review page
`.external-fixtures/render-parity-transparent-high-contrast/visual-review.html`.
The current high-contrast transparent result is alpha mismatches `0`, selected
`rgb-visible` wgpu `53.1994 dB` with max channel delta `1`, and Bevy
`51.9341 dB` with max channel delta `2`; the recipe now enforces selected
`rgb-visible >= 51 dB`, selected RGB channel delta `<= 2`, and exact alpha.
Bevy reaches this by injecting a tiny MToon transparent-order tie-break into
`Transparent3d` before Bevy's phase sort, so equal-depth transparent primitives
no longer depend on incidental ECS/spawn ordering.

For broader transparent material coverage with texture-driven alpha and more
render-queue variation, use:

```powershell
just render-parity-transparent-broad
```

This writes `.external-fixtures/generated/transparent-broad.vrm.gltf` and
renders into `.external-fixtures/render-parity-transparent-broad/`. The fixture
contains four overlapping MToon `BLEND` primitives with mixed
`renderQueueOffsetNumber` values, one `transparentWithZWrite` material, an
embedded base-color PNG whose alpha channel varies per texel, and high-contrast
colors to make layer-order mistakes visible. The recipe keeps
`--render-alpha-mismatch-tolerance 0` while allowing only a 1-LSB
`--render-alpha-channel-tolerance` for browser/GPU rounding. It now also fails
if selected `rgb-visible` falls below `48 dB`, selected RGB channel delta
exceeds `4`, or alpha delta exceeds `1`. The current run has identical alpha
buckets (`transparent=512`, `opaque=0`, `partial=65024`) for three-vrm, wgpu,
and Bevy; all alpha differences are within 1 LSB (`mismatchesBeyondOne = 0`).
Selected `rgb-visible` PSNR is wgpu `48.5282 dB` with max selected channel
delta `3`, and Bevy `48.5944 dB` with max selected channel delta `4`.

For transparent texture-alpha coverage with non-identity glTF
`KHR_texture_transform`, use:

```powershell
just render-parity-transparent-texture-transform
```

This writes
`.external-fixtures/generated/transparent-texture-transform.vrm.gltf` and
renders into
`.external-fixtures/render-parity-transparent-texture-transform/`. The fixture
contains overlapping MToon `BLEND` primitives with mixed render queues,
`transparentWithZWrite`, texture-driven alpha, and base-color texture infos that
apply offset/scale `KHR_texture_transform` values before blending. The recipe
keeps exact alpha-bucket parity, allows only a 2-LSB alpha-channel tolerance for
linear texture-sampling roundoff at transformed texel boundaries, and fails if
selected `rgb-visible` falls below `47 dB`, selected RGB channel delta exceeds
`4`, or alpha delta exceeds `2`. The current run has identical alpha buckets
for all three renderers (`transparent=512`, `opaque=0`, `partial=65024`).
Selected `rgb-visible` PSNR is wgpu `49.1446 dB` with max selected channel
delta `3`, and Bevy `47.2056 dB` with max selected channel delta `4`.

For a broader queue/lighting matrix that combines texture transforms,
`transparentWithZWrite`, forced shade, rim, and emissive strength in one
overlapping stack, use:

```powershell
just render-parity-transparent-queue-matrix
```

This writes `.external-fixtures/generated/transparent-queue-matrix.vrm.gltf`
and renders into `.external-fixtures/render-parity-transparent-queue-matrix/`.
The fixture uses source-generated glTF plus an embedded PNG only; it is intended
to catch ordering or alpha-rounding regressions that pass the narrower
texture-transform and lighted guards in isolation. The recipe uses transparent
background, exact `three-vrm` MToon light accumulation, selected `rgb-visible`,
and a `48 dB` floor while bounding selected RGB channel delta to `<= 4` and
alpha max delta to `<= 2`. The current run has identical alpha buckets
(`transparent=512`, `opaque=0`, `partial=65024`) for all three renderers and
only 1-LSB alpha rounding. Selected `rgb-visible` PSNR is wgpu `53.0342 dB`
with max selected channel delta `2`, and Bevy `48.4839 dB` with max selected
channel delta `3`.

For transparent layers that also exercise MToon lighting, rim color, texture
alpha, `transparentWithZWrite`, and `KHR_materials_emissive_strength`, use:

```powershell
just render-parity-transparent-lighted
```

This writes `.external-fixtures/generated/transparent-lighted.vrm.gltf` and
renders into `.external-fixtures/render-parity-transparent-lighted/`. The
fixture keeps the same source-like embedded-buffer approach as the other
generated transparent guards, but its overlapping `BLEND` layers cover lit
base color, forced shade, parametric rim, and emissive texture/strength
accumulation before transparent blending. The recipe uses the exact
`three-vrm` light accumulator, requires selected `rgb-visible >= 50 dB`, bounds
selected RGB channel delta to `<= 3`, and allows only 2-LSB alpha-channel
rounding. The current run has identical alpha buckets for all three renderers
(`transparent=512`, `opaque=0`, `partial=65024`). Selected `rgb-visible` PSNR
is wgpu `53.7519 dB` with max selected channel delta `2`, and Bevy
`50.2049 dB` with max selected channel delta `3`.

For same-render-order transparent layers at different depths, use:

```powershell
just render-parity-transparent-depth-stack
```

This writes
`.external-fixtures/generated/transparent-depth-stack.vrm.gltf` and renders it
into `.external-fixtures/render-parity-transparent-depth-stack/`. The fixture
contains three MToon `BLEND` primitives with the same `renderQueueOffsetNumber`
but different depths, and one middle layer uses an embedded base-color PNG with
texture alpha. The recipe keeps `--render-alpha-mismatch-tolerance 0` and
allows only 1-LSB alpha channel rounding. It now also fails if selected
`rgb-visible` falls below `49 dB`, selected RGB channel delta exceeds `2`, or
alpha delta exceeds `1`. The current run has identical alpha buckets for all
three renderers (`transparent=31672`, `opaque=0`, `partial=33864`) and no alpha
deltas beyond 1. Selected `rgb-visible` PSNR is wgpu `49.9331 dB` with max
selected channel delta `2`, and Bevy `51.8518 dB` with max selected channel
delta `2`.

For alpha-mode and cutoff coverage across OPAQUE, MASK, and BLEND MToon
materials, use:

```powershell
just render-parity-transparent-alpha-modes
```

This writes
`.external-fixtures/generated/transparent-alpha-modes.vrm.gltf` and renders it
into `.external-fixtures/render-parity-transparent-alpha-modes/`. The fixture
contains four separated swatches: an OPAQUE material whose base alpha must be
forced to `1.0`, a MASK material that passes only because it uses a custom
`alphaCutoff = 0.25`, a MASK material that fails because it uses
`alphaCutoff = 0.70`, and a BLEND material whose `alphaCutoff` must be ignored.
The current run has identical alpha buckets for all three renderers
(`transparent=37904`, `opaque=19184`, `partial=8448`) and zero alpha
mismatches. Selected `rgb-visible` PSNR is wgpu `47.4970 dB` and Bevy
`48.0126 dB`, both with max selected channel delta `2`. The recipe now enforces
selected `rgb-visible >= 47 dB`, selected RGB channel delta `<= 2`, and exact
alpha.

For a generated screen-coordinate outline audit, run:

```powershell
just render-parity-screen-outline-generated
```

This writes `.external-fixtures/generated/screen-outline.vrm.gltf` and renders
it into `.external-fixtures/render-parity-screen-outline-generated/`. The
fixture contains a simple VRM1/MToon prism using
`outlineWidthMode = screenCoordinates`, with a visible outline pass and no
committed binary assets. The recipe uses transparent background plus selected
metric `rgb-opaque`: pixels where both renderers drew the body or outline are
compared for color, while the expected one-pixel fill-rule/silhouette alpha
delta is counted separately. The current result reports alpha mismatches `188`
with tolerance `256`, selected PSNR wgpu `Infinity` with max selected channel
delta `0`, and Bevy `53.2689 dB` with max selected channel delta `1`. Review
`.external-fixtures/render-parity-screen-outline-generated/visual-review.html`
when changing outline expansion, front-face culling, or screen-coordinate width
logic.

To decide whether a new local or downloaded fixture meaningfully broadens
material coverage before adding it to a parity sweep, run:

```powershell
just inspect-mtoon-fixtures
just inspect-mtoon-fixtures .external-fixtures/generated
```

The scanner reads local `.vrm`, `.glb`, and `.gltf` files without committing
them and reports MToon alpha mode, transparent-ZWrite, outline mode, texture
slot, UV-animation coverage, and whether normal-mapped primitives provide glTF
`TANGENT`. The current official fixture inventory has `74` MToon materials, `0`
screen-coordinate outline materials, `4` transparent-ZWrite materials, `3`
UV-animation materials, `13` normal-textured materials, `11` matcap-textured
materials, and `21` normal-mapped primitives without tangents. The generated
fixture inventory adds the missing screen-coordinate outline guard, three
source-like transparent-ZWrite cases, texture-slot guards for shading shift,
rim, UV animation mask, outline width textures, and an opt-in tangentless
normal-map guard. This is why screen-coordinate outline and normal-map
tangentless behavior now have generated parity gates, while real-model
material breadth work should focus on Seed-san, the constraint sample, the
UV-animation sample, and Alicia VRM0. `just render-parity-real-normal-maps`
keeps the unique real tangentless normal-map fixtures in a focused review
directory. A naive derivative normal-map fallback was measured and rejected
because it dropped Seed-san to `20.3102 dB`; the current path uses CPU-generated
tangents with the measured normal-Y convention until a closer three-vrm
derivative formulation improves both the generated normal guard and real fixture
sweep.

## three-vrm Capture

The first reference capture path is a browser script that renders a VRM through
the built three-vrm package and writes the same RGBA JSON format. It keeps
Playwright as an optional local tool rather than a repository dependency:

```powershell
npm install --no-save playwright
node tools\render-parity\three-vrm-browser-capture.mjs `
  --fixture .external-fixtures\official\Seed-san.vrm `
  --three-vrm-root D:\git\three-vrm `
  --out .external-fixtures\render-parity\three-vrm\Seed-san.frame000.rgba.json `
  --width 512 `
  --height 512 `
  --background transparent
```

The script serves the local three-vrm build and fixture through a temporary
localhost server, opens Chromium, renders with a fixed camera/light setup, and
reads RGBA bytes from the WebGL drawing buffer. The Bevy capture path and deeper
wgpu material pass should match this camera/light setup before comparing PSNR.

For human review, also request a PNG:

```powershell
node tools\render-parity\three-vrm-browser-capture.mjs `
  --fixture .external-fixtures\official\Seed-san.vrm `
  --three-vrm-root D:\git\three-vrm `
  --out .external-fixtures\render-parity\three-vrm\Seed-san.frame000.rgba.json `
  --png-out .external-fixtures\render-parity\three-vrm\Seed-san.frame000.png `
  --width 512 `
  --height 512 `
  --camera-z 3.0 `
  --background opaque-black
```

The local smoke run on 2026-05-29 captured Seed-san at `256x256`, wrote a PNG
for visual review, and self-compared the RGBA output with PSNR `Infinity`.
The capture JSON is normalized to top-left row order even though WebGL
`readPixels` returns bottom-left rows, so Bevy/wgpu/ash readbacks can compare
the same row convention.
The three-vrm capture JSON also records a `reference` metadata block with the
Three.js revision, renderer output color space/tone mapping, alpha mode, the
directional and ambient light setup, and the fixed camera frustum. This makes
MToon light/color comparisons traceable even though the three-vrm shader uses
Three.js scene-light accumulation rather than vrm-rs' aggregate capture
uniform.
The Rust capture paths also honor glTF sampler min/mag/wrap policy per texture
using structured sampler data extracted by `vrm-io`; wgpu binds a sampler for
each material texture slot, while Bevy carries the sampler through each image
asset. CPU-generated mip chains use the shared
`vrm-io::image_data_to_rgba8` / `vrm-io::image_bytes_to_rgba8` helpers for
source image normalization and `vrm-io::generate_rgba_mip_chain` with
CatmullRom downsampling for the current capture path, which tracks the WebGL
generated-mipmap reference better than the previous triangle filter on the
official UV-animation and Seed/constraint fixtures. Renderer-facing glTF
material data also exposes
`KHR_materials_unlit`
for non-MToon/PBR fallback materials; when a material has
`VRMC_materials_mtoon`, the concrete captures keep the MToon shader branch even
if glTF unlit is also present, matching the measured three-vrm behavior. This
keeps non-mipmapped VRM0 textures, mipmapped VRM1 textures, generated fixtures,
and mixed glTF/VRMC material extensions on the same sampling/material contract
as GLTFLoader/three-vrm.
CPU-side texture diagnostics now use the shared `vrm-io::CpuRgba8Image`
repeat/linear sampler with an explicit `Rgba8SamplingOrigin`, so renderer
captures can preserve their coordinate-origin choices while sharing the same
outline-width texture sampling math.
Texture slot selection for concrete captures now goes through
`LoadedVrm::material_texture_slots`, which keeps MToon slot lookup and glTF
base/normal fallback behavior identical between wgpu, Bevy, and future
ash/custom renderer examples.
The matching `LoadedVrm::material_uv_transforms` helper now centralizes MToon
texture transforms, glTF texture-transform fallbacks, shade fallback-to-base
behavior, and time-based UV animation scroll/rotation before the concrete
captures apply expression-driven transform overrides.

The canonical comparison images for the current local sample sweep are:

- `.external-fixtures/render-parity/three-vrm/Seed-san.frame000.png`
- `.external-fixtures/render-parity/wgpu/Seed-san.frame000.png`
- `.external-fixtures/render-parity/bevy/Seed-san.frame000.png`
- `.external-fixtures/render-parity/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png`
- `.external-fixtures/render-parity/wgpu/VRM1_Constraint_Twist_Sample.frame000.png`
- `.external-fixtures/render-parity/bevy/VRM1_Constraint_Twist_Sample.frame000.png`
- `.external-fixtures/render-parity/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png`
- `.external-fixtures/render-parity/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png`
- `.external-fixtures/render-parity/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png`

For a time-advanced MToon UV animation check that does not overwrite the static
review set, run:

```powershell
just render-parity-uv-animation
```

This writes `.external-fixtures/render-parity-uv-animation/visual-review.html`
and its renderer artifacts with `--render-mtoon-time 1.0`.

## Renderer Input Data

`vrm-io` now exposes renderer-facing glTF primitive data through
`LoadedVrm::meshes` and node-to-mesh references through `GltfNodeRest::mesh`.
Each `GltfPrimitiveData` stores material index, positions, normals,
`TEXCOORD_0`, and u32 indices. Bevy and wgpu capture paths should build their
mesh buffers from this data so they render the same primitives loaded from the
VRM file rather than using ad hoc sample geometry.

## wgpu Capture

The first Rust renderer path is `examples/wgpu_render_capture.rs`. It is an
offscreen wgpu example, not a new library dependency: it loads a VRM with
`vrm-io`, builds vertex/index buffers from `LoadedVrm::meshes`, renders with the
same fixed camera shape as the three-vrm capture, and writes both RGBA JSON and
an optional PNG:

```powershell
cargo run --example wgpu_render_capture -- `
  --fixture .external-fixtures\official\Seed-san.vrm `
  --out .external-fixtures\render-parity\wgpu\Seed-san.frame000.rgba.json `
  --png-out .external-fixtures\render-parity\wgpu\Seed-san.frame000.png `
  --width 256 `
  --height 256 `
  --camera-z 3.0
```

The local smoke run on 2026-05-29 successfully wrote wgpu RGBA/PNG artifacts
from the real Seed-san mesh primitives. The early textured, rest-skinned PSNR
against the three-vrm reference was `9.70 dB`. After normalizing the three-vrm
readback row order and adding a first MToon-like shade color/toony/shift,
ambient, and emissive pass, the local Seed-san baseline is `20.75 dB`.
After matching the three-vrm camera clip range and using the same effective
directional-light vector convention as three.js, the local Seed-san baseline is
`24.32 dB`. Adding a first world-coordinate MToon outline expansion pass raises
the baseline to `25.89 dB`, and applying outline width multiply textures brings
it to `25.98 dB`. A measured reference-exposure correction for the MToon
lighting approximation raises the baseline to `27.50 dB`. Adding separate
wgpu bindings for MToon `shadeMultiplyTexture` and `matcapTexture` lifts the
current Seed-san baseline to `27.52 dB`; the improvement is small because the
fixture's shade textures mostly alias its main textures, but the renderer path
now exercises a real secondary MToon texture slot. Adding core/sans-IO support
for VRM1 `shadingShiftTexture` scale, applying that red-channel shift in wgpu,
using the three-vrm view-direction matcap UV, and adding parametric rim input
raises the current local Seed-san baseline to `27.63 dB`.
The renderer-facing glTF material data now also exposes alpha mode, alpha
cutoff, and double-sided flags; wgpu and Bevy capture policies consume those
inputs so transparent and double-sided materials do not rely on MToon-only
defaults. It now also exposes glTF emissive factor, emissive texture index, and
`KHR_materials_emissive_strength`; the wgpu capture uses a simple Lambert-style
fallback for non-MToon glTF materials instead of feeding them through the MToon
shader approximation, nudging the Seed-san baseline to `27.64 dB`.
For MToon materials, `vrm-io` mirrors three-vrm's GLTFLoader ordering by
building the ordinary glTF material parameters first and overlaying the
`VRMC_materials_mtoon` extension. When the extension leaves them at default,
the resolved core MToon material inherits glTF `baseColorFactor`,
`baseColorTexture`, `normalTexture`, and `emissiveFactor`; renderer adapters can
therefore consume the core MToon description without separately rejoining those
glTF fields.
The outline draw order now follows the core `MtoonPipelineHints` convention by
placing outline primitives immediately after their base material order; the
current Seed-san frame stays at `27.64 dB`, so this is a pass-order correctness
normalization rather than a measured PSNR improvement on that fixture.
Exposing glTF material `normalTexture` data and primitive `TANGENT` attributes
through `vrm-io`, then sampling tangent-space normal maps in the wgpu capture
raises the current Seed-san baseline to `27.77 dB`. When glTF tangents are
missing, the capture path generates triangle tangents from transformed
positions/UVs and disables normal-map contribution on vertices where no stable
tangent frame can be accumulated.
Matching three-vrm's rim/matcap composition more closely by adding matcap to
the rim term before `rimMultiplyTexture` and `rimLightingMix` raises the current
wgpu Seed-san baseline to `27.98 dB`. Adding CPU-generated mip chains plus a
repeat/linear/mipmap-nearest sampler policy to match the official sample
samplers raises the local Seed-san wgpu baseline again to `28.17 dB`.
Making the capture-only MToon lighting coefficients configurable and retuning
the default exposure/ambient approximation raises the current Seed-san wgpu
baseline to `28.21 dB`.
This is still a failing visual parity baseline: the current path does not yet
apply expression state, screen-space outline details, or exact
three.js/MToon light accumulation.

The first multi-fixture local run also renders
`VRM1_Constraint_Twist_Sample.vrm`. On 2026-05-30, after mipmapped texture
uploads and the lighting-coefficient retune, its wgpu-vs-three-vrm PSNR is
`34.21 dB`, substantially closer than Seed-san because the visible material set
is simpler and less dependent on the remaining MToon deltas.

## Bevy Capture

`examples/bevy_render_capture.rs` is the Bevy 0.18.1 headless renderer path. It
uses Bevy's offscreen `RenderTarget::Image`, a small render-graph copy node,
real `LoadedVrm::meshes`, decoded texture images, and a custom MToon material
for base passes to write the same RGBA JSON plus optional PNG:

```powershell
cargo run --example bevy_render_capture -- `
  --fixture .external-fixtures\official\Seed-san.vrm `
  --out .external-fixtures\render-parity\bevy\Seed-san.frame000.rgba.json `
  --png-out .external-fixtures\render-parity\bevy\Seed-san.frame000.png `
  --width 256 `
  --height 256 `
  --camera-z 3.0
```

The first local Bevy smoke run on 2026-05-29 produced a front-facing Seed-san
capture and a `10.60 dB` baseline against the three-vrm reference. That
historical baseline proved Bevy readback integration worked on Bevy 0.18.1
before the capture path gained skinning, MToon policy, outlines, and a custom
shader.

The next Bevy slice now mirrors the wgpu capture's rest-pose mesh preparation:
glTF node transforms and CPU rest-skinning are baked into the generated Bevy
meshes, and MToon pipeline hints feed `StandardMaterial` alpha, cull,
double-sided, and primitive spawn order. This still uses Bevy's stock material
instead of a custom MToon shader, so it remains a measured baseline rather than
visual parity. After also setting Bevy's perspective projection to the same
30 degree FOV and `0.1..20.0` clip range as the three-vrm/wgpu captures, the
local 2026-05-29 Seed-san Bevy-vs-three-vrm baseline is `23.64 dB`. The alpha
bounding box now matches the three-vrm reference (`252x221` at `256x256`), so
remaining Bevy deltas can be judged as material/runtime differences instead of
camera framing. The current Bevy capture also bakes an approximate MToon
directional-light response into vertex colors while staying on Bevy's stock
`StandardMaterial`, raising the local baseline to `23.77 dB`. Adding the same
expanded outline mesh approach plus outline width multiply texture sampling
raises the Bevy baseline to `24.07 dB`. Applying the same reference-exposure
correction to the baked vertex colors raises it to `24.98 dB`. glTF emissive
factor and `KHR_materials_emissive_strength` now flow into the baked color path
for non-MToon materials, but the current Seed-san frame remains `24.98 dB`
because the visible Bevy delta is still dominated by stock `StandardMaterial`
instead of a custom MToon shader/runtime path. Feeding MToon
`shadingShiftTexture` into the baked vertex-color toon threshold nudges the
current Bevy baseline to `25.00 dB` while preserving Bevy's main texture path.
Bevy outline primitives now use the same base-plus-one pass ordering as the
renderer-agnostic MToon pipeline hints. Spawn order carries the material order
for this capture path; an earlier Bevy material `depth_bias` experiment was
removed after measurement because it was not needed for Seed-san parity and
slightly worsened the captured image.

The current Bevy slice replaces the stock-material bake for base passes with a
custom Bevy 0.18.1 `MaterialPlugin` path and a source-controlled MToon capture
WGSL shader. The material binds base, shade, shading-shift, matcap, rim, and
normal textures plus MToon scalar/color uniforms; normal textures are uploaded
as linear images and sampled when glTF tangents are present. The local
2026-05-30 Seed-san Bevy-vs-three-vrm PSNR was `25.06 dB`. Matching three-vrm's
rim/matcap composition and generating Bevy-side tangents for normal-mapped
primitives that omit glTF `TANGENT` raises the current Seed-san Bevy baseline
to `25.20 dB`. Adding the same CPU-generated mip chains and repeat/linear/
mipmap-nearest sampler policy used by the wgpu capture raises it to `25.26 dB`.
The same configurable lighting defaults bring the current Seed-san Bevy
baseline to `25.27 dB`.
Disabling Bevy camera MSAA to match the antialias-disabled three-vrm and wgpu
captures removes partial alpha edge pixels from the Bevy artifact and raises
the current Seed-san Bevy baseline to `28.00 dB`.
This confirms the custom shader path is wired correctly, but the
small improvement means the next Bevy parity gains need to come from exact
three-vrm MToon light accumulation, runtime expression/pose state, and
screen/clip-space outline behavior rather than more `StandardMaterial` tuning.
The first multi-fixture local run gives
`VRM1_Constraint_Twist_Sample.vrm` a Bevy-vs-three-vrm PSNR of `33.88 dB`, which
confirms the Bevy capture path works beyond Seed-san but remains below the
MToon-lit threshold.

## Review Criteria

Initial thresholds are intentionally conservative until real renderer captures
are stable:

- Static pose, unlit or neutral-light setup: PSNR >= 45 dB.
- MToon lit material render with antialiasing disabled: PSNR >= 40 dB.
- Spring/VRMA animated frame with deterministic camera and fixed timestep:
  PSNR >= 38 dB.

Any failure should store the expected, actual, difference/heatmap image if
available, and the PSNR report under `.external-fixtures/render-parity/reports/`.
Human visual review should compare the rendered PNGs and heatmaps alongside the
numeric report before declaring parity. The local render runner writes
`.external-fixtures/render-parity/visual-review.html` for this review loop.
The runner treats the background alpha policy as part of the contract:
three-vrm, wgpu, and Bevy captures are written through the same Rust PNG
encoder, and the wgpu and Bevy alpha masks are checked against the three-vrm
reference before PSNR is reported. Canonical parity now uses
`--render-background opaque-black` so all three preview PNGs have the same
opaque review background; transparent capture remains available with
`--render-background transparent` for alpha-mask and silhouette audits.
When `tools/render-parity/three-vrm-browser-capture.mjs` is run directly with
`--png-out`, it also writes PNG bytes from the raw `gl.readPixels` RGBA buffer
instead of from a browser canvas data URL, keeping the three-vrm preview PNG
consistent with the selected `--background` mode.
The wgpu and Bevy capture examples build their mesh and skinning matrices from
a shared headless runtime scene after a zero-delta `VrmRuntimeDriver` tick, so
the static render path exercises the same constraint ordering, spring-rest
capture, MToon pipeline hint, emissive-strength, first-person, and VRM0
orientation adapter path as downstream engines.
Outline geometry now also uses the MToon shader path in both capture examples:
after calculating the lit base color, the fragment applies
`outlineColorFactor * mix(1, litColor, outlineLightingMixFactor)`, matching the
shape of three-vrm's outline fragment instead of using a flat unlit outline
material. The 2026-05-30 official-sample sweep after this change is `Seed-san`
wgpu `28.3645 dB`, `Seed-san` Bevy `28.1909 dB`, constraint sample wgpu
`34.2969 dB`, and constraint sample Bevy `34.2969 dB`.
The 2026-05-30 alpha check for that same sweep reports transparent/opaque/partial
counts of `52648/12888/0` for three-vrm Seed-san, `52643/12893/0` for wgpu
Seed-san, and `52644/12892/0` for Bevy Seed-san. The constraint sample reports
`55050/10486/0` for three-vrm and `55055/10481/0` for both wgpu and Bevy.
Double-sided materials now also carry a capture-shader flag so both wgpu and
Bevy flip normals/TBN for back-facing fragments, matching three.js' MToon
double-sided normal path.
The wgpu and Bevy captures now also consume glTF emissive textures and
metallic/roughness factors for non-MToon fallback materials. The fallback is
still intentionally compact, but it uses a GGX-style direct specular term
instead of treating `MeshStandardMaterial` inputs as pure Lambert. The
2026-05-30 sweep after this change reports Seed-san wgpu `28.4228 dB`,
Seed-san Bevy `28.2468 dB`, and unchanged all-MToon constraint values of
wgpu/Bevy `34.2969 dB`.
After glTF base/emissive/texture parameters are merged into resolved MToon
materials, the all-MToon constraint sample improves to wgpu/Bevy `34.3346 dB`
while Seed-san remains wgpu `28.4228 dB` and Bevy `28.2468 dB`. The current
opaque-black six-fixture sample sweep reports `transparent/opaque/partial =
0/65536/0` for three-vrm, wgpu, and Bevy on every fixture, with alpha
mismatches `0`. The capture paths now use CatmullRom mip-chain downsampling for
the generated mip levels, and extract `KHR_materials_unlit` for glTF PBR
fallback materials while keeping VRMC MToon materials on the MToon branch when
both extensions are present. The selected `rgb-visible` metric for the same
sweep is Seed-san wgpu `34.6181 dB`, Seed-san Bevy `34.0835 dB`, constraint
sample wgpu `36.2443 dB`, constraint sample Bevy `36.2352 dB`, UV animation
sample wgpu `35.5223 dB`, UV animation sample Bevy `35.5023 dB`, mask samples
around `55.2-55.7 dB`, and Alicia VRM0 wgpu `35.6238 dB` / Bevy
`35.6088 dB`. Use explicit `transparent` only when the review needs transparent
alpha. The time `1.0` UV-animation audit reports selected `rgb-visible` wgpu
`35.5223 dB` and Bevy `35.5023 dB`.
For the broader real transparent-material audit, run
`just render-parity-real-transparent`. This keeps the transparent background
and uses `rgb-all` with fail-under `32`, while alpha-mask drift is enforced
separately with tolerance `64`. The current six-fixture run passes with
selected PSNR Seed-san wgpu `34.5647 dB`/Bevy `34.0434 dB`, constraint wgpu
`36.2028 dB`/Bevy `36.1877 dB`, UV animation wgpu `35.2517 dB`/Bevy
`35.2073 dB`, expression mask samples around `39.29-39.30 dB`, and Alicia
VRM0 wgpu `32.6863 dB`/Bevy `32.6757 dB`. Alpha mismatches remain below the
tolerance: Seed-san `25/32`, constraint `11/11`, UV animation `0/0`,
expression masks `12/12`, and Alicia `32/32`.
Static `KHR_texture_transform` data is now retained through the non-rendering
layers: VRMC MToon texture infos round-trip nested transform extensions,
`MtoonTextureTransformSet` stores slot-specific transforms in core, and
`vrm-io` extracts glTF base/normal/emissive texture transforms and merges them
into resolved MToon materials. The current two official render fixtures only
exercise identity transforms, so this does not move their PSNR by itself. The
wgpu and Bevy capture paths now consume the retained transforms: wgpu binds a
per-primitive UV-transform uniform beside its material textures, Bevy includes
the same slot-specific transforms in its custom material uniform, and both
shader paths sample base, shade, shading-shift, normal, rim, and emissive
textures through those transforms. Outline-width texture sampling applies the
transform on the CPU side while baking the outline mesh. The capture harness
now also passes MToon material time into three-vrm, wgpu, and Bevy; the wgpu and
Bevy shader paths apply UV animation scroll/rotation before slot-specific
texture transforms and bind the UV-animation-mask texture. The official UV
animation sample at `--render-mtoon-time 1.0` reports wgpu `35.0181 dB` and
Bevy `34.9716 dB` with alpha mismatches `0`. The remaining texture-animation gap is broader
mask-texture fixture coverage and raising the MToon-lit PSNR threshold.
Screen-coordinate outline scaling is also implemented in the concrete capture
paths: when a material requests `outlineWidthMode = screenCoordinates`, the CPU
outline mesh expansion multiplies width by view depth divided by the fixed
30 degree projection Y scale, matching three-vrm's vertex shader convention.
Skinned outline vertices are expanded after the blended skin transform along
the skinned normal direction; unskinned primitives keep the local/object normal
offset path so non-uniform object transforms keep matching the shader formula.
The current official sweep does not exercise that branch, so the remaining
outline gap is fixture breadth plus exact edge/color parity rather than a
missing mode in wgpu/Bevy captures.
Renderer-facing morph target data is also available to the concrete capture
paths. `vrm-io` retains mesh default weights, node weights, and per-target
position/normal/tangent deltas; the wgpu and Bevy captures apply active node
weights before CPU skinning and outline expansion. The render harness also
accepts expression weights and maps resolved VRM expression morph binds into
those active weights, matching three-vrm's `expressionManager.setValue()`
reference path. The 2026-05-30 real tangentless normal/outline sweep using the
exact light accumulator reports Seed-san wgpu `34.5647 dB`, Seed-san Bevy
`34.0434 dB`, constraint wgpu `36.2028 dB`, and constraint Bevy `36.1877 dB`,
so the current real-model
residual is not explained by omitted default morph targets in the static frame.
The generated morph-expression guard closes the previously unmeasured non-zero
target-weight render path.
Generated transparent MToon blend coverage now includes RGB accumulation as
well as alpha buckets. The capture shaders manually sRGB-encode fragment output
into `Rgba8Unorm` targets so blending happens after the same color correction
stage as three-vrm's reference path. The fixture now also embeds a tiny PNG
base-color texture through a glTF bufferView, broadening the transparent path
without committing binary assets. The 2026-05-30
`just render-parity-transparent-generated` run reports `transparent=512`,
`opaque=0`, `partial=65024`, alpha mismatches `0`, wgpu `rgb-visible =
53.0238 dB` with max channel delta `1`, and Bevy `rgb-visible = 49.7151 dB`
with max channel delta `2`. This closes the generated source-like transparent
texture/material blocker. The broader generated transparent run extends this
with texture alpha, four mixed-queue layers, and `transparentWithZWrite`; it
passes with exact alpha buckets, no alpha deltas beyond 1 LSB, and selected
`rgb-visible` PSNR wgpu `48.5282 dB` / Bevy `48.5944 dB`. The remaining
transparent work is broader real-fixture coverage and raising the current
real transparent sweep beyond its low `rgb-all` regression floor.
That generated fixture now also carries a `COLOR_0` gradient. This intentionally
does not change the MToon reference image: three-vrm ignores vertex colors for
MToon materials, and the Rust capture paths must do the same. `vrm-io` still
extracts `COLOR_0` for renderer-facing consumers and the wgpu capture applies
it to non-MToon/PBR fallback materials, but MToon color accumulation keeps
vertex colors out of base and shade color.
VRM0 compatibility shading now includes the three-vrm `V0_COMPAT_SHADE` branch:
wgpu and Bevy capture materials flag VRM0 MToon primitives through the spare
`emissive.w` lane, and the shader clamps the direct toon contribution with
`min(direct, diffuse)`. The current six-fixture sweep is stable after this
exactness fix. The capture light vector now also follows three-vrm's
`DirectionalLight(1,1,1)` convention after applying the VRM orientation
compensation, which raises the current six-fixture `rgb-visible` floor to
`32 dB`. The remaining PSNR gap is no longer the isolated base/shade/rim/matcap
formula; the heatmaps now point more toward runtime/material breadth, outline
edges, real-model screen-coordinate outline coverage, and higher thresholds.

## Next Renderer Work

- Deepen real-model runtime/material breadth now that isolated MToon
  light/color, angled-normal ramp, tangentless normal-map, MToon
  occlusion-ignore, and VRM0 compat shade guards are covered by generated or
  official parity runs.
- Add or discover an external screen-coordinate outline fixture so the newly
  implemented screen-width path is measured against three-vrm instead of only
  being compile/render-path covered.
- Add or discover broader transparent-material fixtures with real textures and
  mixed render queues so the generated blend fixture and current six-fixture
  `rgb-all` transparent sweep are not the only transparent RGB parity guards.
- Review real tangentless official normal-map primitives and raise the normal
  generated fixture floor beyond the current `45 dB` when stable; the focused
  real normal-map sweep now enforces `rgb-visible >= 34 dB`.
- For Bevy specifically, deepen the new custom MToon material/shader path
  instead of returning to `StandardMaterial` vertex-color baking.
- Use the generated heatmaps to prioritize remaining outline, material breadth,
  and real-model shader deltas before raising thresholds again.
