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
  --fail-under 40
```

The report contains dimensions, MSE, PSNR, maximum channel delta, maximum pixel
delta, alpha counts/mismatches, RGB-only opaque/visible/interior metrics, and
pass/fail status. Exact matches report `"Infinity"` for PSNR. The pass/fail
threshold still uses the full RGBA PSNR; the RGB-only fields are diagnostic
helpers for separating alpha/edge disagreement from opaque-surface shading.

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
  --render-fixture Seed-san.vrm `
  --render-fixture VRM1_Constraint_Twist_Sample.vrm
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
the same Rust PNG writer. The canonical run uses `--render-background
opaque-black`, so the three-vrm reference PNG, wgpu PNG, and Bevy PNG all use
the same opaque background/alpha contract. `--render-background transparent`
remains available for targeted alpha-background investigations. This keeps
preview PNGs aligned with the exact RGBA buffers used for PSNR instead of
relying on browser element screenshots or canvas compositing. At the start of
each render-parity run, the managed `three-vrm`, `wgpu`, `bevy`, `reports`, and
`diff` directories are recreated so older direct-capture smoke images cannot be
mistaken for the current canonical comparison set. Each PNG is decoded after
writing and must match its RGBA artifact bytes, including alpha.

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
  --background opaque-black
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
  --camera-z 3.0
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
reference before PSNR is reported. Canonical parity uses an opaque black
background because the three-vrm reference path is not reliably reviewable as a
transparent PNG across tools; transparent capture remains available with
`--render-background transparent`.
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
while Seed-san remains wgpu `28.4228 dB` and Bevy `28.2468 dB`. Switching the
canonical render-parity background to opaque black removes the remaining
background-alpha mismatch from the visual review artifacts: the current
two-fixture sweep reports `transparent/opaque/partial = 0/65536/0` for
three-vrm, wgpu, and Bevy on both samples, with alpha mismatches `0`. Current
PSNR values are Seed-san wgpu `28.7208 dB`, Seed-san Bevy `28.6162 dB`, and
constraint sample wgpu/Bevy `34.8595 dB`.
Static `KHR_texture_transform` data is now retained through the non-rendering
layers: VRMC MToon texture infos round-trip nested transform extensions,
`MtoonTextureTransformSet` stores slot-specific transforms in core, and
`vrm-io` extracts glTF base/normal/emissive texture transforms and merges them
into resolved MToon materials. The current two official render fixtures only
exercise identity transforms, so this does not move their PSNR by itself. The
next renderer slice should consume `MtoonTextureTransformSet` in the wgpu and
Bevy capture shaders, including separate UVs for base, shade, normal, rim,
emissive, outline-width, and UV-animation-mask texture slots.

## Next Renderer Work

- Deepen the wgpu and Bevy capture paths with fuller MToon lighting/color
  accumulation, screen-coordinate outline behavior, and material/shader parity
  before raising PSNR thresholds.
- For Bevy specifically, deepen the new custom MToon material/shader path
  instead of returning to `StandardMaterial` vertex-color baking.
- Use the generated heatmaps to prioritize the remaining MToon shader deltas.
