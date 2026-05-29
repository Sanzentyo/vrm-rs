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
delta, and pass/fail status. Exact matches report `"Infinity"` for PSNR.

For the full local Seed-san parity loop, use the Rust local CI runner:

```powershell
cargo +nightly -Zscript tools/ci/local-ci.rs -- `
  --render-parity `
  --skip-core `
  --skip-coverage `
  --skip-download `
  --skip-three-vrm-build `
  --skip-playwright-install `
  --three-vrm-root D:\git\three-vrm
```

Without the `--skip-*` flags, the same script can download external fixtures,
prepare Playwright, and build the pinned three-vrm checkout under
`.external-fixtures/three-vrm`. The render pass writes:

- `.external-fixtures/render-parity/three-vrm/Seed-san.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/wgpu/Seed-san.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/bevy/Seed-san.frame000.{rgba.json,png}`
- `.external-fixtures/render-parity/reports/Seed-san.{wgpu,bevy}-vs-three-vrm.psnr.json`
- `.external-fixtures/render-parity/visual-review.html`
- `.external-fixtures/render-parity/diff/Seed-san.{wgpu,bevy}-vs-three-vrm.diff.png`

Open `visual-review.html` locally to compare the three PNGs side-by-side with
their PSNR reports and diff heatmaps. In the heatmaps, red shows RGB-channel
delta and blue shows alpha-channel delta, amplified for review. It is generated
data and stays outside git.

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
  --height 512
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
This is still a failing visual parity baseline: the current path does not yet
apply expression state, Bevy-side normal/custom MToon shading, screen-space
outline details, or exact three.js/MToon light accumulation.

## Bevy Capture

`examples/bevy_render_capture.rs` is the first Bevy 0.18.1 headless renderer
path. It uses Bevy's offscreen `RenderTarget::Image`, a small render-graph
copy node, real `LoadedVrm::meshes`, decoded texture images, and an unlit
`StandardMaterial` baseline to write the same RGBA JSON plus optional PNG:

```powershell
cargo run --example bevy_render_capture -- `
  --fixture .external-fixtures\official\Seed-san.vrm `
  --out .external-fixtures\render-parity\bevy\Seed-san.frame000.rgba.json `
  --png-out .external-fixtures\render-parity\bevy\Seed-san.frame000.png `
  --width 256 `
  --height 256 `
  --camera-z 3.0
```

The local Bevy smoke run on 2026-05-29 produced a front-facing Seed-san
capture and a first PSNR baseline of `10.60 dB` against the three-vrm
reference. This is an intentionally failing renderer baseline: it proves Bevy
readback integration works on Bevy 0.18.1, but it does not yet apply skinning,
MToon shading, material render ordering, outlines, or expression/runtime state.

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
renderer-agnostic MToon pipeline hints. The local Seed-san PSNR remains
`25.00 dB`, but the example no longer relies on equal-order stable sorting for
base/outline sequencing.

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

## Next Renderer Work

- Deepen the wgpu and Bevy capture paths with fuller MToon lighting and
  expression/runtime state before raising PSNR thresholds.
- Use the generated heatmaps to prioritize the remaining MToon/runtime deltas.
