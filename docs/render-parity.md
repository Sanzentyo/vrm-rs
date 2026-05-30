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
`--metric rgb-visible`, and `--metric rgb-interior1px`; pass/fail thresholds
use the selected metric. The local render-parity runner defaults to
`rgb-visible`; with the canonical opaque-black review background this is the
visible RGB surface metric, and it also remains useful for explicit transparent
alpha-mask audits.

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
  --render-mtoon-light-accumulation tuned `
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
- `.external-fixtures/render-parity/visual-review.html`
- `.external-fixtures/render-parity/diff/<fixture>.{wgpu,bevy}-vs-three-vrm.diff.png`

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
`rgb-visible` with a low `--render-fail-under 20` regression floor. That floor
does not mean final visual compatibility is complete; it keeps the current
real-fixture transparent path from regressing while generated transparent
fixtures continue to enforce stricter alpha/blend behavior. The current run
keeps alpha mismatches under `64` for all six fixtures: Seed-san wgpu/Bevy
`25/32`, constraint `11/11`, UV animation `0/0`, expression mask samples
`12/12`, and Alicia VRM0 `32/32`. Selected `rgb-visible` PSNR is Seed-san wgpu
`20.4130 dB`, Seed-san Bevy `20.3065 dB`, constraint wgpu `25.6531 dB`,
constraint Bevy `25.6476 dB`, UV animation wgpu `27.4437 dB`, UV animation
Bevy `27.4271 dB`, expression mask samples around `29.84 dB`, Alicia wgpu
`20.3469 dB`, and Alicia Bevy `20.3434 dB`. Review
`.external-fixtures/render-parity-real-transparent/visual-review.html` when
working on broader transparent silhouettes, VRM0 material ordering, or runtime
pose/material breadth.

The Rust capture paths also accept
`--mtoon-light-accumulation three-vrm`. The default `tuned` mode keeps the
current PSNR-oriented ambient proxy (`ambientBase + ambientGiScale * gi`).
`three-vrm` mode uses the closer WebGL MToon accumulator shape for light/color
audits: direct diffuse is normalized by the `DirectionalLight(Math.PI)` setup,
indirect diffuse uses `pbrAmbient`, rim lighting uses the same
direct-plus-indirect accumulator (`1.0 + pbrAmbient`) as the three-vrm
non-physical-light path, and the final exposure multiplier is fixed at `1.0`
instead of inheriting the tuned `0.78` capture coefficient. Re-run that focused
path with:

```powershell
just render-parity-light-three-vrm
```

For a generated MToon light/color accumulation audit that isolates shader
terms without relying on redistributable binary model assets, run:

```powershell
just render-parity-mtoon-light-generated
```

This writes `.external-fixtures/generated/mtoon-light.vrm.gltf` and renders it
into `.external-fixtures/render-parity-mtoon-light-generated/`. The fixture
contains six opaque MToon quads for forced base lighting, forced shade lighting,
ambient-only behavior, parametric rim, matcap rim, and mixed rim/matcap. It
uses `--mtoon-light-accumulation three-vrm`,
`--render-background transparent`, and selected metric `rgb-interior1px` so a
one-pixel silhouette/rasterization disagreement does not dominate the shader
color score. The current run reports selected PSNR wgpu `54.4445 dB` with max
selected channel delta `1`, and Bevy `51.0870 dB` with max selected channel
delta `2`. The expected edge-only alpha mismatch is `240` pixels and the recipe
allows up to `512`; use the strict transparent generated fixture below for
alpha/blend correctness. Review
`.external-fixtures/render-parity-mtoon-light-generated/visual-review.html`
when changing MToon accumulation code. The remaining broad-fixture PSNR gap is
therefore more likely to come from real model runtime/material breadth,
post-correction, outline edges, and fixture coverage than from this isolated
base/shade/rim/matcap color formula.

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
alpha mismatch tolerance is intentionally zero for this generated fixture.
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
`51.9341 dB` with max channel delta `2`. Bevy reaches this by injecting a tiny
MToon transparent-order tie-break into `Transparent3d` before Bevy's phase sort,
so equal-depth transparent primitives no longer depend on incidental ECS/spawn
ordering.

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
mismatches `0`. Full-RGBA PSNR is Seed-san wgpu `28.7208 dB`, Seed-san Bevy
`28.6162 dB`, constraint sample wgpu/Bevy `34.8595 dB`, UV animation sample
wgpu/Bevy `36.1575 dB`, mask samples wgpu/Bevy `40.5553 dB` and `40.5557 dB`,
and Alicia VRM0 wgpu/Bevy `29.0481 dB`. The selected `rgb-visible` metric for
the same sweep is Seed-san wgpu `27.4714 dB`, Seed-san Bevy `27.3668 dB`,
constraint sample wgpu/Bevy `33.6101 dB`, UV animation sample wgpu/Bevy
`34.9081 dB`, mask samples wgpu/Bevy `39.3059 dB` and `39.3063 dB`, and Alicia
VRM0 wgpu/Bevy `27.7987 dB`. Use explicit `transparent` only when the review
needs transparent alpha. The last transparent time `1.0` UV-animation audit
reported full-RGBA wgpu/Bevy `35.9209 dB` and selected `rgb-visible`
wgpu/Bevy `27.2167 dB`.
For the broader real transparent-material audit, run
`just render-parity-real-transparent`. This keeps the transparent background
and uses `rgb-all` with fail-under `27`, while alpha-mask drift is enforced
separately with tolerance `64`. The current six-fixture run passes with
selected PSNR Seed-san wgpu `27.4709 dB`/Bevy `27.3634 dB`, constraint wgpu
`33.6106 dB`/Bevy `33.6050 dB`, UV animation wgpu `34.8985 dB`/Bevy
`34.8819 dB`, expression mask samples around `39.29-39.30 dB`, and Alicia
VRM0 wgpu `27.7958 dB`/Bevy `27.7923 dB`. Alpha mismatches remain below the
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
animation sample at `--render-mtoon-time 1.0` reports wgpu/Bevy `35.9209 dB`
with alpha mismatches `0`. The remaining texture-animation gap is broader
mask-texture fixture coverage and raising the MToon-lit PSNR threshold.
Screen-coordinate outline scaling is also implemented in the concrete capture
paths: when a material requests `outlineWidthMode = screenCoordinates`, the CPU
outline mesh expansion multiplies width by view depth divided by the fixed
30 degree projection Y scale, matching three-vrm's vertex shader convention.
The current official sweep does not exercise that branch, so the remaining
outline gap is fixture breadth plus exact edge/color parity rather than a
missing mode in wgpu/Bevy captures.
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
texture/material blocker; the remaining transparent work is broader real-fixture
coverage and high-contrast transparent layer ordering in Bevy.
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
exactness fix, so the remaining PSNR gap is not this branch; the heatmaps still
point at broader MToon light/color accumulation, outline edges, and fixture
coverage.

## Next Renderer Work

- Deepen the wgpu and Bevy capture paths with fuller MToon lighting/color
  accumulation beyond the now-covered VRM0 compat shade clamp
  before raising PSNR thresholds.
- Add or discover an external screen-coordinate outline fixture so the newly
  implemented screen-width path is measured against three-vrm instead of only
  being compile/render-path covered.
- Add or discover broader transparent-material fixtures with real textures and
  mixed render queues so the generated blend fixture and current six-fixture
  `rgb-all` transparent sweep are not the only transparent RGB parity guards.
- For Bevy specifically, deepen the new custom MToon material/shader path
  instead of returning to `StandardMaterial` vertex-color baking.
- Use the generated heatmaps to prioritize the remaining MToon shader deltas.
