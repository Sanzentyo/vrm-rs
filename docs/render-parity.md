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

The capture paths can also write `imqraw` directly:

```powershell
cargo run --example wgpu_render_capture -- `
  --fixture .external-fixtures/generated/mtoon-light.vrm.gltf `
  --out .external-fixtures/render-parity-imqraw-smoke/wgpu/mtoon-light.frame000.rgba.json `
  --imqraw-out .external-fixtures/render-parity-imqraw-smoke/wgpu/mtoon-light.frame000.imqraw `
  --width 64 `
  --height 64 `
  --background transparent

imq bundle-info .external-fixtures/render-parity-imqraw-smoke/wgpu/mtoon-light.frame000.imqraw --format json
```

The three-vrm browser reference accepts the same kind of direct raw artifact:

```powershell
node tools/render-parity/three-vrm-browser-capture.mjs `
  --fixture .external-fixtures/official/Seed-san.vrm `
  --three-vrm-root .external-fixtures/three-vrm `
  --out .external-fixtures/render-parity-imqraw-smoke/three-vrm/Seed-san.frame000.rgba.json `
  --imqraw-out .external-fixtures/render-parity-imqraw-smoke/three-vrm/Seed-san.frame000.imqraw `
  --width 64 `
  --height 64
```

The ash example is not yet a full visual-parity renderer, but its offscreen
Vulkan readback now emits the same raw artifact shape:

```powershell
just ash-render-parity-readback
```

This writes `.external-fixtures/ash-readback-smoke/ash/Seed-san.frame000.rgba.json`
and `.external-fixtures/ash-readback-smoke/ash/Seed-san.frame000.imqraw`, then
uses `verify-imqraw-rgba.rs` to prove both files carry identical RGBA bytes.
Use this as the ash-side bridge into future direct raw comparisons before
raising it to the same visual threshold path as wgpu and Bevy.

The local render-parity runner can include that bridge beside the normal
three-vrm/wgpu/Bevy capture set:

```powershell
just render-parity-with-ash-readback
```

This forwards `--render-ash-readback` to `tools/ci/local-ci.rs`. The ash
artifacts are written under the selected render-parity directory as
`ash/<fixture>.frame000.{rgba.json,imqraw,png}`, verified with the same
imqraw/RGBA byte check, compared against the three-vrm reference with the same
RGBA and direct `.imqraw` report writers used by wgpu and Bevy, and recorded in
`review-manifest.json` as a `comparisons[]` entry with `visualParityGate: false`.
The local runner compiles the source-controlled Ash MToon GLSL handoff into
SPIR-V under `target/render-parity-ash-mtoon-shaders` and passes it through
`unsafe_device_renderer --vertex-spv --fragment-spv`, so the review path no
longer compares the built-in color-smoke shader. The same runner also forwards
the render camera distance, direct-light scale, directional color, and MToon
lighting knobs into the Ash frame plan. The current Ash path also applies the
Vulkan clip-space Y flip/front-face pairing, bakes base color into vertices,
generates missing tangents, carries per-vertex normal scale and double-sided
state, binds slot-specific fallback textures, and uses the same UV animation
rotation direction as the wgpu/Bevy capture shader before readback. Ash also
accepts the same render-parity `--diagnostic-render` modes and applies the same
VRM model-orientation boundary to baked world/skinning matrices. It is still
non-gating: the latest focused Seed-san review artifact is
`target/render-parity-ash-review-128-oriented-diagnostics`, where Ash reaches
`15.4876 dB` selected `rgb-visible` PSNR with exact opaque alpha parity while
wgpu/Bevy remain above `32 dB`. The oriented diagnostic artifacts are
`target/render-parity-ash-diagnostic-flat-128-oriented` (`22.3215 dB`) and
`target/render-parity-ash-diagnostic-base-color-128-oriented` (`19.3898 dB`).
Ash owner-id diagnostics now bake triangle IDs into vertex color after
draw-order sorting; use `target/render-parity-ash-diagnostic-owner-id-128-sorted`
(`14.6654 dB`) for the current primitive-ownership review.
The Ash source shader now matches the wgpu capture shader's direct-light
multiplier shape; the 128px shaded artifact
`target/render-parity-ash-review-128-direct-scale` stays at `15.4876 dB`. Do
not remove the current Ash fragment outline-width mask as a blind wgpu
alignment step: the 2026-06-19 check under
`target/render-parity-ash-review-128-outline-color-direct` fell to `15.4337 dB`,
so the remaining outline blocker is coupled to Ash geometry/fill behavior.
Ash now also accepts the same normal-map diagnostic axis as wgpu/Bevy:
`--normal-map-mode generated-tangents|derivative|view-derivative`,
`--normal-map-scale`, and `--disable-normal-maps` are forwarded through the
local render-parity runner into the release-built Vulkan readback example. The
GLSL handoff shader implements the derivative and view-derivative fallback path
using `dFdx`/`dFdy` and the material-extra view-derivative flag. On Seed-san the
2026-06-19 diagnostic runs
`target/render-parity-ash-review-128-normal-derivative` and
`target/render-parity-ash-review-128-normal-view-derivative` both reported Ash
selected `rgb-visible` PSNR `15.4683 dB`, slightly below the current generated
tangent default `15.4876 dB`, so this is a parity investigation knob rather
than the current default path.
The same parity runner now forwards `--disable-outlines`,
`--outline-width-scale`, and `--render-mtoon-time` to Ash. The 2026-06-19
outline-off smoke under
`target/render-parity-ash-review-128-outline-off-forwarded` confirms the
release-built Vulkan example records `21` draw plans instead of `31`, with
exact opaque alpha parity and Ash selected `rgb-visible` PSNR `16.6624 dB`.
This makes Ash outline diagnostics comparable to wgpu/Bevy/three-vrm without
making outline-off the default path.
Ash descriptor plans now also derive Vulkan sampler filter, wrap, mipmap mode,
and LOD clamps from the glTF texture sampler referenced by each material
texture slot. This aligns the Ash resource handoff with the wgpu/Bevy capture
paths before further material-color work. The 2026-06-19 Seed-san smoke under
`target/render-parity-ash-review-128-sampler-policy` preserved exact opaque
alpha parity and stayed at Ash selected `rgb-visible` PSNR `15.4876 dB`, so the
change is a compatibility prerequisite rather than the current visible blocker.
Keep `--render-ash-visual-gate` opt-in until the Ash GLSL/resource path gains
the remaining wgpu-equivalent outline/primitive edge coverage and MToon
material accumulation behavior.
Pass `--render-ash-visual-gate` with `--render-ash-readback` to apply the same
fail-under and max-delta threshold arguments to Ash once the current
texture/material color parity gap is closed.

For the current source-controlled Ash MToon base shader handoff, run:

```powershell
just ash-mtoon-base-readback
```

This compiles `crates/vrm-adapter-ash/shaders/mtoon_base.{vert,frag}.glsl`
with `glslangValidator`, writes local SPIR-V under `target/`, feeds those
modules into `unsafe_device_renderer --vertex-spv --fragment-spv`, and verifies
the direct `.imqraw` bundle against the `.rgba.json` readback artifact. The
shader now consumes the shared MToon uniform ABI, fixed texture slots,
normal/tangent vertex attributes, UV animation, shade/shading-shift textures,
alpha mode, outline-pass flag, and a frame-level scene uniform for
view-projection, camera position, light direction/color, and MToon lighting
accumulation. It also exposes the same packed material-extra and UV-transform
uniform surfaces used by the wgpu and Bevy captures, so transformed base, shade,
shading-shift, normal, matcap, rim, and UV-animation-mask sampling can be driven
through the same IO-derived plans. The fragment shader also uses the closer
capture-side MToon accumulation shape: linear-to-SRGB output correction,
view-space matcap UVs, glTF emissive texture multiplication, PBR/MToon ambient
occlusion sampling, unlit and PBR fallback branches, v0-compatible direct light
clamping, rim plus matcap composition, and render-extra direct-light scaling. It
now also emits expanded outline primitives through the same renderer-neutral
outline helper used by the wgpu and Bevy captures, and routes those draws to the
outline pipeline rather than the base pipeline. It is still not the final visual
parity shader: Ash is now visible in the full PSNR/visual review harness, but it
remains non-gating by default until its color accumulation reaches the wgpu/Bevy
threshold floor. The current 128x128 Seed-san smoke has exact opaque-black alpha
parity and selected `rgb-visible` Ash PSNR `12.7715 dB` after the camera/light
sync and Vulkan Y-flip fixes.

`tools/ci/local-ci.rs --render-parity` now asks three-vrm, wgpu, Bevy, and
optionally Ash to emit `.frame000.imqraw` beside their `.rgba.json` artifacts.
The current public
`imq image` CLI still does not expose the VRM-specific selected-metric gates
needed by this repository, so the local runner uses the Rust comparator below
as the authoritative direct-raw numeric gate.

For direct renderer raw-buffer checks with the same VRM-specific metric domains
as `compare-psnr.mjs`, use the Rust imqraw comparator:

```powershell
cargo +nightly -Zscript tools/render-parity/compare-imqraw.rs `
  --expected .external-fixtures/render-parity/three-vrm/Seed-san.frame000.imqraw `
  --actual .external-fixtures/render-parity/wgpu/Seed-san.frame000.imqraw `
  --metric rgb-visible `
  --out .external-fixtures/render-parity/reports/Seed-san.wgpu-vs-three-vrm.imqraw-rust.json
```

`tools/ci/local-ci.rs --render-parity` writes this direct-imqraw report beside
the existing `.psnr.json` report as `<fixture>.<renderer>-vs-three-vrm.imqraw-rust.json`.
The pass/fail summary consumes the `.imqraw-rust.json` report. The older
`.psnr.json` report remains a diagnostic cross-check over the renderer
`.rgba.json` artifacts and is embedded in `visual-review.html`.

For direct raw-buffer hotspot inspection, use:

```powershell
just imqraw-deltas `
  .external-fixtures/render-parity-real-normal-maps/three-vrm/Seed-san.frame000.imqraw `
  .external-fixtures/render-parity-real-normal-maps/wgpu/Seed-san.frame000.imqraw `
  .external-fixtures/render-parity-real-normal-maps/reports/Seed-san.wgpu-vs-three-vrm.deltas.json `
  32 `
  1 `
  shared-nonblack-interior1px
```

`tools/render-parity/inspect-imqraw-deltas.rs` reports the worst per-pixel RGBA
deltas, changed-pixel bounds, and whether each changed pixel is visible,
nonblack, one-pixel-interior, actual-only, expected-only, or shared-nonblack
inside a one/two/three-pixel body mask. Use `actual-only` / `expected-only` for
coverage-only pixels and `shared-nonblack-interior1px` or
`shared-nonblack-interior2px` for material/UV/body-color deltas that should
ignore silhouette and background classification noise. Use
`shared-nonblack-interior3px` for the stricter Seed-san base-UV residual slice
after edge ownership has already been identified. The convenience recipe
`just render-parity-imqraw-seed-normal-deltas` writes wgpu and Bevy reports for
the current real normal-map Seed-san artifacts. The 2026-06-10 Seed-san report
shows no alpha deltas, but max RGB delta `255` inside visible/interior pixels:
wgpu has `8151` changed pixels with `6791` interior-nonblack, and Bevy has
`10340` changed pixels with `8855` interior-nonblack. That points the remaining
Seed-san blocker at model-body material/geometry/pose residuals rather than PNG
encoding, alpha, or transparent-background handling.

For a stronger geometry/pose versus material-color split, run the flat
diagnostic:

```powershell
just render-parity-seed-flat-diagnostic
```

This forwards `--render-diagnostic-mode flat` to three-vrm, wgpu, and Bevy.
The mode keeps the same mesh transforms, skinning, outline/cull/depth policy,
opaque alpha contract, and render order, but paints fragments white after the
renderer-side alpha/cutoff branch. It is intentionally a Seed-san-oriented
opaque diagnostic, not a replacement for normal MToon or texture-alpha parity.
The 2026-06-10 Seed-san run writes
`.external-fixtures/render-parity-seed-flat-diagnostic/` and reports
`rgb-nonblack-interior1px = Infinity` for both wgpu and Bevy. The direct
hotspot reports have `0` `interiorNonblackChangedPixels`; wgpu has `229`
changed pixels and Bevy has `228`, all RGB-only silhouette/raster-edge pixels.
This narrows the main Seed-san body residual to material/shader color parity
while keeping edge/raster differences as a separate blocker.

For the next material/shader split, run:

```powershell
just render-parity-seed-base-factor-diagnostic
just render-parity-seed-base-color-diagnostic
just render-parity-seed-base-color-raw-srgb-diagnostic
just render-parity-seed-base-color-interior2-diagnostic
just render-parity-seed-base-color-no-mip-diagnostic
just render-parity-seed-base-color-flip-v-diagnostic
just render-parity-seed-uv-diagnostic
just render-parity-seed-base-uv-diagnostic
```

`base-factor` keeps the same alpha/cull/depth/order policy and paints fragments
with the resolved material base color factor only. `base-color` multiplies that
factor by the resolved main/base texture, but still skips MToon light, normal,
rim, shade, and emissive terms. Both recipes use
`rgb-shared-nonblack-interior1px`, which measures only one-pixel-interior RGB
pixels where both the three-vrm reference and Rust capture drew nonblack model
content. This drops opaque-black background dilution and silhouette/raster-edge
classification noise while preserving body-color deltas.

The 2026-06-10 Seed-san diagnostic writes
`.external-fixtures/render-parity-seed-base-factor-diagnostic/` and reports
base-factor selected PSNR `Infinity` for wgpu and `74.8665 dB` for Bevy, with
max selected-channel deltas `0` and `1`. The matching base-color run writes
`.external-fixtures/render-parity-seed-base-color-diagnostic/` and reports
wgpu `32.4602 dB` / Bevy `32.4177 dB`, with max selected-channel deltas `219`
/ `218`. This confirms that material assignment and base factors match in the
model-body overlap region; the remaining Seed-san color blocker starts at
base-texture sampling, UV/sampler state, texture color-space handling, or a
texture-selection detail before the MToon lighting stack is applied.

`render-parity-seed-base-color-raw-srgb-diagnostic` is a Rust-only experiment:
the three-vrm reference still renders its normal `base-color` diagnostic, while
wgpu/Bevy bind only the base texture as raw `RGBA8Unorm` and manually apply
shader sRGB decode. The current run reports wgpu `32.1018 dB` / Bevy
`32.0621 dB`, slightly worse than the normal sRGB-resource path. Because the
generated texture-boundary and texture-selection guards still pass on the
normal path, this raw-base experiment should stay diagnostic rather than
becoming the renderer default.

Two follow-up diagnostics keep that blocker narrower. The
`render-parity-seed-base-color-interior2-diagnostic` recipe uses the stricter
`rgb-shared-nonblack-interior2px` metric and still only rises to wgpu
`30.0100 dB` / Bevy `28.8790 dB`, with large worst-case selected-channel
deltas intact. The `render-parity-seed-base-color-no-mip-diagnostic` recipe
disables texture mipmaps in three-vrm, wgpu, and Bevy; it worsens to wgpu
`28.5038 dB` / Bevy `27.6273 dB`, so mip usage itself is not the primary
blocker. The `render-parity-seed-base-color-flip-v-diagnostic` recipe
samples the Rust base texture with flipped V coordinates while leaving the
three-vrm reference unchanged; it worsens sharply to wgpu `10.9506 dB` / Bevy
`10.9418 dB`. Together these rule out a simple one-pixel edge mask or global
V-flip explanation. The current evidence points at localized texture sampling,
UV discontinuity, sampler state, or per-primitive texture-selection
behavior. Mean RGB over the shared body pixels remains close, so this is not a
global color-space bias.

The `render-parity-seed-uv-diagnostic` recipe renders UV coordinates as color
using the same primitive, skin, morph, alpha, cull, depth, and order path. It
reports selected `rgb-shared-nonblack-interior1px` PSNR wgpu `30.3816 dB` /
Bevy `30.1784 dB`, with max selected-channel deltas `213` / `214`. This is
better than the textured base-color diagnostic, but still in the same localized
residual band, so the next Seed-san slice should inspect UV interpolation,
primitive coverage, sampler state, and texture lookup locality before returning
to MToon light/color accumulation.

The `render-parity-seed-base-uv-diagnostic` recipe renders the transformed base
texture sampling UV instead of raw `TEXCOORD_0`. On Seed-san it currently
matches the raw UV diagnostic exactly, which rules out base texture transform
and MToon UV animation as the focused base-texture cause for this fixture.

To map those raw delta pixels back to model geometry, run:

```powershell
just render-parity-seed-base-uv-hotspots
```

This writes
`.external-fixtures/render-parity-seed-base-uv-diagnostic/reports/Seed-san.{wgpu,bevy}-vs-three-vrm.hotspots.json`.
`tools/render-parity/map-render-hotspots.rs` reuses the renderer-independent
`vrm-io` transformed vertex path and the same fixed capture camera to list
candidate node, mesh, primitive, material, material pipeline policy, triangle,
raw UV, and transformed base UV values for each top direct-imqraw hotspot
pixel. The report also records nearest-by-encoded-UV matches for both all
candidate faces and faces that pass the material cull policy, decoding the
sRGB diagnostic color back to linear UV before comparing against geometry UVs.
It also reports the visible frontmost candidate from the Rust-side projected
geometry, which helps avoid misreading back-face CPU hits or color-space encoded
UVs as render-visible winners. Use it after
`just render-parity-seed-base-uv-diagnostic` when the next question is "which
material/primitive owns this residual?" The mapper defaults to the local
render-parity runner's camera (`256x256`, `camera-z = 3`) and to non-expanded
diagnostic outlines, matching three-vrm's diagnostic material replacement. Pass
`--expand-outlines` when mapping normal shaded artifacts, and pass `--width`,
`--height`, or camera options when inspecting artifacts generated with custom
capture settings.

The 2026-06-10 focused Seed-san hotspot pass shows wgpu and Bevy producing the
same top-32 structure. After sRGB-to-linear decode and perspective-correct UV
interpolation, Rust's actual diagnostic color matches the mapper's
`frontmost_visible` candidate for nearly every top hotspot. The three-vrm
expected color usually maps to a nearby visible candidate with a one-pixel
sample offset: expected offsets are spread across `1,0` (`12/32`), `0,-1`
(`6/32`), `0,1` (`6/32`), `-1,0` (`5/32`), and only `0,0` (`2/32`). This rules
out a single global viewport shift. All dominant candidates are opaque,
depth-writing, back-face-culled base passes, so the remaining base-UV blocker is
best treated as local triangle-boundary / UV-seam raster selection between the
three-vrm WebGL path and the Rust CPU-prepared geometry, rather than
transparent blending, outline expansion, color-space decoding, or texture
transform state.

The hotspot JSON now carries a `summary` block so this classification can be
tracked without external PowerShell grouping. The current top-32 summary is:
wgpu actual/frontmost triangle matches `31/32`, Bevy actual/frontmost triangle
matches `27/32`, and both renderers have expected/frontmost triangle matches
only `4/32`. The mean frontmost UV distance is tiny for Rust actuals
(`0.0017` wgpu, `0.0039` Bevy) but large for the three-vrm expected colors
(`0.3739`), confirming that the diagnostic is measuring a reference-vs-Rust
surface-selection difference rather than random texture sampling drift.

For base-color diagnostics, the same mapper also samples the frontmost
material's base texture through `vrm-io::CpuRgba8Image` and writes
`frontmost_base_texture_rgba`,
`frontmost_base_texture_expected_rgb_distance`, and
`frontmost_base_texture_actual_rgb_distance`. On the current Seed-san
base-color hotspot report, the sampled frontmost base texture is close to the
Rust actual color (mean RGB distance about `18`) but far from the three-vrm
expected color (mean distance about `320`). Treat that as evidence against a
broad base texture binding, V-flip, or generated sampler failure, and focus the
next investigation on real-model-local surface selection, visibility, or
diagnostic material ownership near texture/UV boundaries.

The mapper's `visible_by_policy` classification includes cull policy and mask
alpha discard. Candidate entries expose `alpha`, `visible_by_cull_policy`, and
`visible_by_alpha_policy`; the summary exposes rejected candidate counts. On
the same Seed-san base-color top-32 reports, wgpu and Bevy both have `329`
candidates rejected by cull policy and `0` rejected by alpha policy. That rules
out alpha-mask discard as the focused hotspot cause and points the next slice at
cull/coverage/surface ownership.

For cull-specific checks, the report also includes `frontmost_any` and
`frontmost_alpha_visible` next to `frontmost_visible`. These use the same center
sample and depth ordering while ignoring cull, or ignoring cull but preserving
alpha-mask discard. On the current Seed-san base-color top-32 reports, all
three counts are `20/32` and `frontmost_any_cull_rejected_count` is `0`, so the
missing `12/32` frontmost pixels are not recovered by disabling cull. Treat the
remaining focused blocker as center coverage / primitive ownership / diagnostic
surface ownership around real model UV boundaries.

For center-coverage checks, the report includes
`nearest_sample_visible_frontmost` and
`missing_center_nearest_visible_offsets`. These search the configured hit
radius after the center sample fails. On the current Seed-san base-color top-32
reports, the missing `12/32` center hits are all recovered within the 1px
neighborhood, with offsets distributed across left, up, down, and right. The
recovered neighbors' base texture colors are close to three-vrm expected
(missing-center mean RGB distance `33.05`) and far from Rust actual (mean
`378.13`). That is the strongest current sign that the focused residual is
raster/sample ownership at local UV or material boundaries.

When investigating the actual base-color PSNR gate, prefer the shared-body
hotspot recipe:

```powershell
just render-parity-seed-base-color-hotspots-focused
just render-parity-seed-base-color-three-hotspots
just render-parity-seed-base-color-three-hotspots D:/git/three-vrm 0.75 0.75
just render-parity-seed-base-color-owner-hotspots
just render-parity-seed-base-color-owner-hotspots D:/git/three-vrm 0.75 0.75
just render-parity-seed-owner-id-diagnostic
just render-parity-seed-owner-id-front-face-cw-diagnostic D:/git/three-vrm
```

It creates `shared-nonblack-interior1px` delta reports and maps the top 64
pixels, matching the render-parity metric rather than the all-pixel diagnostic
domain. The current focused reports have center visible candidates for all
top-64 pixels, and the CPU-sampled frontmost base texture is closer to Rust
actual than three-vrm expected. Use that focused report when working on the
remaining shared-body PSNR floor; use the broader all-pixel report for
silhouette and raster edge ownership.

`render-parity-seed-base-color-three-hotspots` sends the same shared-body delta
pixels back through the browser after three-vrm has loaded the avatar. Its
projection report includes CPU-sampled `material.map` colors in
`reference.renderer.diagnosticHotspots.*.projectedBaseColorSrgb`. At sample
center `0.5,0.5`, the browser-side CPU frontmost candidate is closer to Rust
actual than to the rendered three-vrm expected color (mean RGB distance
`52.06` vs `115.12`). The nearest candidate to the expected color is closer
(`43.65` mean distance), but it is the same surface as frontmost only `24/64`
times. At `0.75,0.75`, expected/frontmost improves (`99.28`) while
actual/frontmost worsens (`74.28`), matching the earlier conclusion that this
is a local fill/depth/surface ownership issue rather than a global sample-center
offset.

`render-parity-seed-base-color-owner-hotspots` renders the same three-vrm scene
with `--diagnostic-render owner-id`. The browser diagnostic replaces each
diagnostic triangle with a stable RGB owner ID, decodes the WebGL-rendered
owner at each hotspot pixel, and compares it with the CPU-projected candidate
set in `reference.renderer.diagnosticHotspots`. The report includes a
`summary` block plus per-pixel `renderedOwnerRecovery` records for a 3x3
same-pixel subpixel grid and a 3x3 one-pixel-neighborhood grid. The recipe now
also runs `tools/render-parity/summarize-owner-hotspots.rs`, writing compact
JSON/Markdown summaries under
`.external-fixtures/render-parity-seed-base-color-diagnostic/reports/Seed-san.owner-hotspots.*.summary.{json,md}`.
Those summaries include rendered-owner material counts and rendered-to-frontmost
/ rendered-to-recovered material and triangle transitions, so repeated
diagnostic runs no longer require ad hoc JSON snippets. On the current
Seed-san top-64
shared-body base-color hotspots, every pixel has a WebGL owner, but that owner
appears in the CPU center-sample candidate set only `20/64` times at
`0.5,0.5` and `18/64` times at `0.75,0.75`. Owner/frontmost matches are only
`14/64` and `17/64`. The same-pixel subpixel grid recovers `34/64` rendered
owners, all as frontmost, with best centers split across `0.5,0.5`,
`0.75,0.5`, `0.25,0.5`, and smaller vertical offsets. The one-pixel
neighborhood grid recovers `31/64`, `30/64` as frontmost, with offsets spread
across center/down/right/left/up. This is the current strongest evidence that
the active base-color blocker is a local WebGL fill/raster ownership issue
near real material/UV boundaries rather than texture binding, global
color-space, alpha-mask, cull, mip, or sample-center policy.

`render-parity-seed-owner-id-diagnostic` renders owner IDs through the normal
three-vrm/wgpu/Bevy local render-parity runner. The three-vrm reference and
concrete Rust captures now both use per-triangle owner IDs for this diagnostic.
The Rust captures de-index triangle lists only in `owner-id` mode and carry the
diagnostic color through vertex color attributes, leaving the normal shaded
paths unchanged. Therefore the three-vrm-vs-Rust PSNR is still a diagnostic
shape, not a compatibility threshold, but it now isolates draw-order and local
triangle ownership rather than a coarse primitive/pass ID mismatch. On the
2026-06-10 Seed-san run, three-vrm-vs-wgpu reports selected `rgb-visible`
`16.6869 dB` and three-vrm-vs-Bevy reports `16.7007 dB`, both with alpha
mismatches `0`. wgpu-vs-Bevy is no longer byte-identical because triangle-edge
ownership is visible, but remains close at `62.1850 dB` with max channel delta
`2`; use that as the renderer-internal sanity check while investigating the
remaining WebGL-vs-Rust per-triangle ownership mapping.

The same recipe also writes owner-specific direct raw reports with
`tools/render-parity/compare-owner-id-images.rs`:

```powershell
.external-fixtures/render-parity-seed-owner-id-diagnostic/reports/Seed-san.wgpu-vs-three-vrm.owner-ids.json
.external-fixtures/render-parity-seed-owner-id-diagnostic/reports/Seed-san.bevy-vs-three-vrm.owner-ids.json
.external-fixtures/render-parity-seed-owner-id-diagnostic/reports/Seed-san.bevy-vs-wgpu.owner-ids.json
```

These reports decode the diagnostic RGB values into owner IDs and summarize
same-pixel matches, one-pixel-neighborhood recovery, top owner transitions, and
top `expected - actual` ID offset clusters. The current three-vrm-vs-Rust
reports have `0` exact owner matches because the owner streams are assigned
independently, but the dominant offsets are now explicit: wgpu has a `+36404`
cluster for `6966` shared-nonzero pixels, while Bevy has `+36404` for `3138`
and `+36660` for `1779`. The wgpu-vs-Bevy report is the tighter renderer
sanity check: `12665` shared nonzero pixels, `6341` exact owner matches, and
the largest residual offsets are `+256`, `+1`, and `-1`. Use this before
changing triangle generation/order; a useful change should shrink those offset
clusters or make their cause obvious.

The owner-specific reports become labeled when adjacent `.rgba.json` artifacts
contain diagnostic owner metadata. The wgpu and Bevy captures now write that
metadata under `renderer.diagnosticOwnerIds`; each label includes owner
ID/color, node index, mesh index, primitive index, material index/name,
base-vs-outline pass, render order, triangle ordinal, and source indices. The
comparator reads both normal Rust metadata and three-vrm reference metadata
from `/renderer/diagnosticOwnerIds` or `/reference/renderer/diagnosticOwnerIds`
and writes `top_expected_to_actual_details` plus
`top_actual_to_expected_details`. Those detail records also include the pixel
`bounds` of the transition, up to eight `sample_pixels`, each owner's
projected `screenBounds`, renderer-native NDC `depth`, normalized
`webglDepth`, depth range, screen signed area, a screen-area front-facing flag,
and render policy fields such as material side/cull mode, depth write/test,
blend, alpha mode, render order, and draw index when the capture artifact
contains enough triangle metadata. On the current Seed-san owner run, both Rust
captures emit `81236` owner labels. Rust owner labels also include glTF node
and mesh names from `vrm-io` rest data. The leading three-vrm-vs-Rust
transition maps three-vrm `arm_plastic (Outline)` on mesh `robo_arm_2` to Rust
base-pass `material_6`, node/mesh `robo_arm`, node `144`, mesh `3`, primitive
`1`, with the same local triangle ordinal band (`406/407`). Its transition
pixels are bounded by `x=187..209`, `y=58..65`; the expected and actual
triangle screen bounds overlap to float precision (`x=186.98..210.80`,
`y=57.80..65.74`). The renderer-native depth values differ because three-vrm
reports WebGL -1..1 NDC while Rust reports 0..1 NDC, but `webglDepth` matches
after normalization (`0.9489916` three-vrm versus `0.9489918` Rust). The same
triangle is back-facing under the three.js screen-area convention
(`screenSignedArea ~= -185.816`, `frontFacing=false`): three-vrm renders the
BackSide outline owner (`side=1`, `drawIndex=40236`, `renderOrder=19`), while
Rust reports the base owner (`cullMode=back`, `drawIndex=6`, `renderOrder=2000`)
and also has a matching outline owner later in its stream (`cullMode=front`,
`drawIndex=24`) at the same screen bounds/depth. The leading wgpu-vs-Bevy
transitions are narrower: same material/pass/node/mesh/primitive, but
neighboring triangle ordinals (`442 -> 443`, `418 -> 417`, `666 -> 665`) and
the existing `+256` cluster. Treat this as evidence that the next useful parity
step is cull/facing/pass ownership alignment between the three-vrm WebGL draw
stream and Rust's sorted triangle stream, with wgpu-vs-Bevy triangle-edge
differences as a secondary sanity check.

The same reports include `top_pass_transitions` so pass ownership changes can
be tracked without reading every owner pair. The three-vrm browser `owner-id`
reference now builds a diagnostic geometry by duplicating all attributes and
morph attributes per material group. This matters for MToon meshes such as
Seed-san, where base and outline are overlapping material groups over the same
index range. The old single non-indexed geometry shared one vertex-color stream
across both groups, so outline owner IDs could be drawn during the base material
pass. After the group-local duplication fix, the default Seed-san owner
diagnostic keeps the Rust captures on `frontFace = ccw`, matching the shaded
renderer default, and reports wgpu pass transitions `base -> base = 11911`,
`outline -> outline = 18`, and `outline -> base = 4`. The direct wgpu-vs-Bevy
owner report remains a backend sanity check, while the old large `outline ->
base` transition is now classified as a browser-reference diagnostic artifact.

They also include `top_render_policy_transitions`, which groups the same
mismatched shared-owner pixels by expected pass/material side/front-facing/
depth-write and actual pass/cull/front-face/front-facing/depth-write. The
three-vrm side now also emits effective three.js-style `frontFace`, `cullMode`,
`gpuFrontFacing`, and `visibleByCullPolicy`, accounting for `BackSide` and
negative world determinants. Recomputing the default CCW Seed-san wgpu report
shows the leading policy bucket as
`base/back/ccw/gpuFrontFacing=true/visibleByCullPolicy=true -> base/back/ccw/gpuFrontFacing=true/visibleByCullPolicy=true`
(`11911` pixels). Only `18` pixels remain in the same-pass outline bucket and
`4` in `outline -> base`. The CW diagnostic now worsens, dominated by
`base -> outline = 11913`, so keep CW as a negative diagnostic rather than a
candidate renderer default.

The Rust captures now add `gpuFrontFacing` and `visibleByCullPolicy` alongside
the older `frontFacing` field. `frontFacing` is the shared screen-area label
used to compare with the three-vrm browser metadata. `gpuFrontFacing` is the
Rust capture's estimate after applying the selected `frontFace` convention to
the y-down screen-space winding, and `visibleByCullPolicy` applies the
primitive cull mode to that estimate. Prefer these fields when asking
"should this pipeline have drawn this triangle?" Bevy still shows buckets where
the actual-side metadata estimate says not visible. `compare-owner-id-images.rs`
also reports
`actual_not_visible_by_cull_policy_shared_nonzero`,
`actual_not_visible_by_cull_policy_mismatched_shared_nonzero`, and
`top_actual_cull_visibility` so this split can be read without scanning the
full policy table. With the fixed browser reference, wgpu remains at `0`
actual-not-visible shared owner pixels in both default CCW and CW diagnostics.
Bevy reports `2436` default CCW pixels and `1014` CW pixels in both the shared
and mismatched-shared slices, so use wgpu as the cleaner pass/facing probe and
treat the Bevy split as a separate projection/fill, color-decode, or
specialization diagnostic until it is explained.
The Bevy capture now computes this owner metadata projection through Bevy's own
`PerspectiveProjection::get_clip_from_view()` path and labels its depth range
as `bevy-reverse-zero-to-one-ndc`; the split did not change after moving from
the older finite-projection approximation. That rules out projection depth as
the explanation for the Bevy-only metadata-visibility bucket, but not
edge/fill, color decode, or material specialization effects.

The owner comparator also reports actual-side metadata bounds misses and
near-ID recovery:
`actual_metadata_bounds_miss_shared_nonzero`,
`actual_metadata_bounds_miss_mismatched_shared_nonzero`,
`actual_metadata_bounds_miss_recovered_by_near_id_shared_nonzero`,
`actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero`,
and `top_actual_metadata_recoveries`. On the current default CCW Seed-san run,
wgpu-vs-three-vrm has `0` metadata misses. Bevy has `4498` metadata misses in
wgpu-vs-Bevy and all `4498` recover to a nearby owner ID whose screen bounds
contain the pixel; Bevy-vs-three-vrm is `4492/4492`. Treat this as a Bevy
diagnostic-ID/fill ownership artifact, not as evidence that Bevy drew culled
triangles. The three-vrm-vs-wgpu report remains the cleaner signal for the
remaining pass/material/fill parity work.

For the clean wgpu probe, prefer the geometry-class fields before reading raw
owner-ID PSNR. `compare-owner-id-images.rs` reports
`same_projected_triangle_mismatched_shared_nonzero`,
`same_projected_or_adjacent_triangle_mismatched_shared_nonzero`, and
`top_owner_geometry_classes`. These classify mismatched owner pixels by pass
relation, normalized mesh-name relation, material relation, triangle relation,
and projected screen/depth relation. The same report also exposes
`unexplained_owner_tail_mismatched_shared_nonzero` and
`top_unexplained_expected_to_actual_details`, which remove same/adjacent
projected-triangle mismatches and Bevy near-ID recovery artifacts from the
detail table. On the current default CCW Seed-san run, `11649/11933`
wgpu-vs-three-vrm mismatched shared pixels are still the same projected
triangle, and `11866/11933` are the same, adjacent, or shared-edge projected
triangle. The largest bucket is same pass / same normalized mesh / different
material label / same triangle / depth-close projection (`11590` pixels), which
mainly reflects owner-numbering and three.js clone/material naming differences.
The clean wgpu unexplained tail is now `67/11933`; `198` pixels previously read
as different-triangle ownership are now classified as `shared-edge-indices`.
The useful remaining owner work is therefore the small non-overlap,
different-triangle, or different-pass tail rather than the whole owner-ID PSNR
delta. Use wgpu for this tail; Bevy reports are still useful for consistency
but include reverse-Z metadata convention and near-ID diagnostic-color recovery
effects.

The owner comparator also reports
`same_projected_or_touching_triangle_mismatched_shared_nonzero` and
`unexplained_owner_tail_after_touching_mismatched_shared_nonzero`. This
diagnostic adds indexed shared vertices to the stricter same/adjacent/shared-edge
set, so it should be read as a local-topology residual lens rather than an exact
parity pass condition. On the current Seed-san outline-off reports, wgpu-vs-three-vrm
has `11853/11933` touching-local mismatches and `80` pixels left after touching
classification, Bevy-vs-three-vrm has `7977/12140` and `149`, and Bevy-vs-wgpu
has `2228/6332` and `99`.

When auditing Bevy owner reports, keep the strict close-depth fields separate
from near-depth convention checks. `same_projected_or_adjacent_triangle_*`
still requires `overlap-depth-close` (`<= 0.001` WebGL-reference depth), while
`same_projected_or_adjacent_triangle_near_depth_mismatched_shared_nonzero`
also counts `overlap-depth-near` (`<= 0.02`). The projection-gap summary records
`within_webgl_depth_001` and `within_webgl_depth_02` so the size of the depth
offset is visible without reclassifying it as exact parity. Bevy 0.18 uses
infinite reverse-Z for its actual camera projection, while the three-vrm/wgpu
reference uses finite WebGL-style depth. The Bevy capture therefore writes both
actual `webglDepth` / `depthRange` and comparison-oriented
`referenceWebglDepth` / `referenceDepthRange`; `compare-owner-id-images.rs`
prefers the reference field when present. On the current Seed-san outline-off
report, Bevy has `7292/12140` mismatched shared pixels that are strictly
same/adjacent/shared-edge projected triangles, `7303/12140` within `0.02`
depth, and its unexplained tail is down to `539`; wgpu's corresponding tail is
only `85` pixels, with `79` already within `0.001`.

For cull/facing isolation, run
`just render-parity-seed-owner-id-front-face-cw-diagnostic D:/git/three-vrm`.
It forwards `--render-front-face cw` only to the Rust wgpu/Bevy capture paths
and records `Front face: cw` in the generated summary/manifest and owner label
metadata. After fixing group-local owner colors in the browser reference, CW is
a negative diagnostic: Seed-san wgpu reports `base -> outline = 11913`,
`base -> base = 712`, `outline -> base = 18`, and `outline -> outline = 4`;
selected owner-ID PSNR drops to wgpu `14.4732 dB` and Bevy `14.4760 dB`.
Do not use this as the shaded default. Treat the CW recipe as a regression
guard for front-face experiments before changing material or lighting code.

For subpixel raster-convention checks, `map-render-hotspots.rs` accepts
`--sample-center-x` and `--sample-center-y` while leaving the default at the
normal pixel center `0.5,0.5`. On the current Seed-san top-32 base-UV hotspots,
the default center best explains Rust actuals (`31/32` wgpu and `27/32` Bevy
actual/frontmost triangle matches). A `0.75,0.50` center best explains the
three-vrm expected colors in both reports (`16/32` expected/frontmost triangle
matches, expected mean UV distance `0.15`), and `0.75,0.75` lowers expected
mean UV distance further to `0.13` with the same `16/32` expected triangle
matches. This suggests a subpixel raster-alignment component mixed into the
internal UV seam residual, but not a complete global offset: choosing that
center worsens Rust actual/frontmost agreement.

The follow-up Rust-side screen-jitter capture confirms that the sample-center
hint should not be applied as a global projection offset. The capture examples
and local runner accept `--screen-jitter-x/y` for focused diagnostics; the wgpu
path applies a tested clip-space projection offset, while the Bevy path uses a
fixed-camera translation approximation for capture-only comparison. On Seed-san base-UV,
`rgb-shared-nonblack-interior1px` baseline is wgpu `38.6925 dB` / Bevy
`38.5360 dB` with max selected-channel delta `166`. Jittering Rust by `+0.25`
pixel on X drops the score to wgpu `27.0500 dB` / Bevy `26.9913 dB`, and
`-0.25` drops it to wgpu `26.9579 dB` / Bevy `26.9061 dB`. Treat the
sample-center mapper result as a classification clue, not a render correction;
the remaining blocker is still local GPU/CPU-prepared surface selection around
UV seams and triangle boundaries.

`map-render-hotspots.rs` now records each candidate's minimum barycentric value
and nearest screen-space triangle-edge distance in pixels. Re-running the
Seed-san base-UV hotspot maps shows the current top-32 frontmost candidates are
all within `0.25px` of a triangle edge for both wgpu and Bevy, with mean edge
distance `0.0341px`. For the current top hotspot set, this makes boundary
ownership the strongest explanation; broader interior pixels and lower-amplitude
deltas still need separate checks before ruling out every sampler or
interpolation edge case.

For that broader check, run:

```powershell
just render-parity-seed-base-uv-hotspots-wide
```

This maps the top-256 `shared-nonblack-interior1px` base-UV deltas. The wider
2026-06-10 pass keeps the same shape but is less absolute than top-32: wgpu has
`173/256` frontmost candidates within `0.25px` of an edge, `217/256` within
`0.5px`, and `253/256` within `1px`; Bevy has `149/256`, `204/256`, and
`246/256` respectively. Mean edge distance is wgpu `0.2257px` and Bevy
`0.2924px`. So the worst deltas sit directly on triangle boundaries, while the
lower-amplitude band is still mostly a near-boundary phenomenon rather than a
large model-interior drift.

The hotspot mapper also reports `frontmost_nearest_edge_counts`, which groups
frontmost samples by node/mesh/primitive/material/triangle/edge. On the 2026-06-10
top-32 pass, wgpu and Bevy report the same leading edge buckets: node `145`,
mesh `4`, primitive `3`, material `1`, triangles `160`, `164`, and `171` each
account for three of the worst samples, followed by node `144`, mesh `3`,
primitive `0`, material `5`, triangle `1548` with two samples. The top-256 pass
spreads into more mesh `3` and mesh `4` base-pass edges, but remains concentrated
around local triangle ownership rather than a global camera, color, or sampler
offset. This makes adjacent-triangle/depth-edge policy the next most useful
diagnostic target.

The same mapper now reports whether the closest actual/expected visible UV
candidate is an immediate neighbor across the frontmost sample's nearest edge.
On the top-32 pass, wgpu is actual `31/32` same-triangle and expected `4/32`
same-triangle, but actual/expected edge-neighbor matches are `0/0`; Bevy is
actual `27/32`, expected `4/32`, and edge-neighbor `1/0`. The top-256 pass
shows more adjacency but still not enough to explain the residual by itself:
wgpu actual/expected same-triangle `208/185` and edge-neighbor `10/17`; Bevy
same-triangle `145/180` and edge-neighbor `39/25`. This weakens the simple
"three-vrm picked the adjacent triangle" hypothesis for the worst pixels and
points next toward local UV/color quantization, interpolation/sample-position,
or non-unique UV-color matching diagnostics.

The mapper now quantizes the CPU frontmost `base_uv` through the same
linear-to-sRGB path used by the wgpu diagnostic shader and reports RGB distance
to the captured actual/expected pixels. This strongly separates Rust renderer
self-consistency from the remaining three-vrm delta: top-32 wgpu has frontmost
RGB mean actual/expected `0.0313/80.0337` with max `1/188.0877`; Bevy has
`0.7134/80.0337` with max `1.4142/188.0877`. The top-256 pass is less extreme
but keeps the same shape: wgpu `0.6093/11.1972`, Bevy `1.8690/10.9681`. That
means the Rust CPU projection, UV transform, and capture shader agree closely
with each other, while three-vrm's diagnostic `vMapUv` path still differs on
the worst pixels. The next high-value target is to inspect or reproduce
three.js `vMapUv` generation for the affected materials, including map matrix,
UV channel, and derivative/sample-position behavior.

`three-vrm-browser-capture.mjs` records per-material diagnostic map metadata in
`reference.renderer.diagnosticMaterials`. Re-running the Seed-san base-UV
diagnostic on 2026-06-10 showed 29 mapped diagnostic materials with identity
map transforms: offset `0,0`, repeat `1,1`, rotation `0`, center `0,0`,
channel `0`, `flipY=false`, and identity texture matrix; two entries have no
map. That rules out a normal KHR/three.js texture-transform mismatch for the
current worst pixels and pushes the remaining investigation toward UV attribute
selection, shader varying behavior, or sample-position/rasterization details.
The same artifact now records `reference.renderer.diagnosticMeshes`; Seed-san
reports 21 diagnostic meshes with `uv` present and no populated `uv1`/`uv2`
attributes, which also weakens a UV-channel mismatch explanation.

For browser-side projection of the same hotspot pixels after three-vrm has
loaded and updated the avatar, run:

```powershell
just render-parity-seed-base-uv-three-hotspots
```

This writes
`.external-fixtures/render-parity-seed-base-uv-diagnostic/three-vrm/Seed-san.hotspot-projection.rgba.json`
with `reference.renderer.diagnosticHotspots`. On the 2026-06-10 top-32 pass,
the three.js CPU projection's frontmost candidate still does not match the
rendered three-vrm pixel (`0/32` exact; mean RGB distance `76.72`), but the
nearest same-pixel candidate improves the match (`5/32` exact, `20/32` within
RGB distance `16`, mean `26.81`). The nearest rendered-color candidates are
often same-mesh triangles behind the CPU-frontmost candidate by small depth
deltas, so the current base-UV residual is now most consistent with edge/fill
rule, sample-position, or rasterization ownership differences rather than
material texture transforms, UV channel selection, or Rust-side scene
projection.

The projection diagnostic accepts `sample_x` / `sample_y` just parameters, which
are passed as `--hotspot-sample-center-x/y`. A 3x3 sweep over `0.25`, `0.5`,
and `0.75` found `0.75,0.75` as the best hotspot-projection center: nearest
candidate mean RGB distance improved from `26.81` at `0.5,0.5` to `11.77`, and
nearest candidates within RGB distance `16` improved from `20/32` to `26/32`.
However, actually jittering Rust rendering by `+0.25,+0.25` pixels worsens the
full base-UV PSNR from wgpu/Bevy `38.14/38.12 dB` to `27.01/27.45 dB`. So the
remaining issue is not a global camera offset; it is local triangle fill /
raster ownership near boundaries.

The local comparator also exposes `rgb-shared-nonblack-interior3px` for this
specific diagnostic. Reusing the same Seed-san base-UV raw captures, the
shared-nonblack interior sweep is:

| metric | wgpu PSNR / MAE | Bevy PSNR / MAE |
| --- | ---: | ---: |
| `rgb-shared-nonblack-interior1px` | `38.6925 dB` / `0.1307` | `38.5360 dB` / `0.4307` |
| `rgb-shared-nonblack-interior2px` | `39.1371 dB` / `0.1204` | `38.9656 dB` / `0.4216` |
| `rgb-shared-nonblack-interior3px` | `40.4952 dB` / `0.1040` | `40.2751 dB` / `0.4050` |

The improvement confirms that the dominant base-UV diagnostic error is near
shared-edge ownership. The maximum selected channel delta remains `166`, so
there are still a few strong localized residuals to inspect before treating
this diagnostic as exhausted.

`just render-parity-seed-base-uv-hotspots-interior3` extracts and maps the
remaining 3px shared-body hotspots. On the current Seed-san run, wgpu has `676`
changed pixels in this domain and Bevy has `5889`. For the top 32 hotspots,
both renderers still match the Rust CPU-projected frontmost diagnostic color
closely while the three-vrm/reference pixel is far away: wgpu actual/reference
frontmost mean RGB distance is `0.0754` / `49.1641`, and Bevy is `0.9047` /
`49.0316`. The top hotspot geometry is still concentrated on the same face
region, with repeated buckets on node `145`, mesh `4`, primitive `3`, material
`1`, triangles `160`, `164`, `171`, `156`, and `179`. The frontmost mean edge
distance is only about `0.063px`, and `30/32` hotspots are within `0.25px` of
the nearest edge for both wgpu and Bevy. This keeps the base-UV residual
classified as local shared-edge/raster ownership even after the 3px body mask.

A source-like generated UV-boundary control is available through:

```powershell
just render-parity-uv-boundary-generated
```

It writes `.external-fixtures/generated/uv-boundary.vrm.gltf` and renders
`base-uv` diagnostics into `.external-fixtures/render-parity-uv-boundary-generated/`.
The fixture uses simple planar panels with an intentional UV discontinuity and
opposing triangle splits, while keeping outlines disabled. The current run
passes a high guard on `rgb-shared-nonblack-interior1px`: wgpu `66.8378 dB`
with max selected-channel delta `1`, and Bevy `53.0706 dB` with max delta `2`.
Its hotspot maps match the frontmost triangle for both expected and actual
colors (`32/32`), so simple generated UV seams do not reproduce the Seed-san
residual. Treat this as a control fixture: the remaining real-model blocker is
more specific than generic planar UV interpolation or a basic triangle split.

A source-like generated base-texture UV-boundary control is available through:

```powershell
just render-parity-texture-boundary-generated
```

It writes `.external-fixtures/generated/texture-boundary.vrm.gltf` and renders
`base-color` diagnostics into
`.external-fixtures/render-parity-texture-boundary-generated/` with outlines
disabled. The fixture reuses the generated UV-boundary topology, adds a small
embedded PNG base-color texture through a glTF bufferView, and keeps a single
opaque MToon material so the result isolates base texture sampling over UV
discontinuities from material assignment and MToon lighting. The current guard
has exact alpha parity and selected `rgb-shared-nonblack-interior1px` PSNR wgpu
`52.9232 dB` / Bevy `49.5132 dB`, with max selected-channel deltas `7` / `6`.
The hotspot maps are useful for geometry ownership only on this fixture: top-32
wgpu/Bevy deltas all agree with the frontmost base-pass triangle and material,
while the remaining RGB differences are small linear-filtering/rounding deltas
spread across the textured surface. This rules out a generic embedded PNG,
sampler, or generated UV-discontinuity failure as the Seed-san base-texture
cause; the real-model blocker remains more specific to Seed-san's local
primitive/texture lookup path.

A source-like per-material base texture selection guard is available through:

```powershell
just render-parity-texture-selection-generated
```

It writes `.external-fixtures/generated/texture-selection.vrm.gltf` and renders
`base-color` diagnostics into
`.external-fixtures/render-parity-texture-selection-generated/` with outlines
disabled. The fixture uses four opaque MToon primitives, four materials, and
four distinct embedded PNG base textures, so a material/texture binding mix-up
would produce a large color error. The current guard has exact alpha parity and
selected `rgb-shared-nonblack-interior1px` PSNR wgpu `58.0021 dB` / Bevy
`52.5998 dB`, with max selected-channel deltas `1` / `2`. The top-32 hotspot
maps match the frontmost base-pass triangle and material for both expected and
actual pixels, so generated per-material base texture selection is now covered.
This narrows the remaining Seed-san base-texture blocker away from broad
texture binding-table mistakes and toward real-model-local UV/coverage/lookup
details.

A source-like generated base-material seam control is available through:

```powershell
just render-parity-material-seam-generated
```

It writes `.external-fixtures/generated/material-seam.vrm.gltf` and renders
`base-factor` diagnostics into
`.external-fixtures/render-parity-material-seam-generated/` with outlines
disabled. The fixture uses adjacent opaque MToon primitives with shared world
coordinates but distinct materials, including a high-contrast diagonal material
boundary. The current run has exact alpha parity and selected
`rgb-shared-nonblack-interior1px` PSNR wgpu `42.1555 dB` / Bevy `42.1555 dB`,
with max selected-channel delta `192`. Direct `.imqraw` hotspot inspection finds
only `7` changed shared-nonblack interior pixels for both Rust renderers. The
hotspot map classifies all of them as base-pass pixels within `0.25px` of the
diagonal material seam, with the dominant transition `material_2 ->
material_3`. Treat this as the minimal fill-rule/material-ownership guard that
corresponds to the larger real constraint-sample seam transitions.

A source-like same-material subpixel ownership control is available through:

```powershell
just render-parity-subpixel-ownership-generated
just render-parity-subpixel-ownership-owner-generated
just render-parity-subpixel-ownership-owner-hotspots
```

It writes `.external-fixtures/generated/subpixel-ownership.vrm.gltf` and keeps
the material name `huku_bake` while placing high-contrast texture regions across
near-subpixel triangle seams. The base-color diagnostic currently has exact
alpha parity, selected `rgb-shared-nonblack-interior1px` PSNR wgpu
`43.8282 dB` / Bevy `43.4195 dB`, and max selected-channel deltas `161` /
`160`. The new owner-id recipe writes
`.external-fixtures/render-parity-subpixel-ownership-owner-generated/`; on the
2026-06-19 run, wgpu and Bevy owner-id images matched exactly, and each differed
from three-vrm by only `2` of `18276` shared nonzero owner pixels. The hotspot
owner projection under
`.external-fixtures/render-parity-subpixel-ownership-generated/` found all
`32/32` top hotspot rendered owners on `huku_bake`, `28/32` already on the
center frontmost triangle, and `32/32` recovered to frontmost rank 1 through
subpixel or one-pixel-neighbor search. This makes the generated fixture a
stable guard for subpixel owner diagnostics while keeping Seed-san's remaining
gradient-domain blocker classified as a more complex real-topology/UV ownership
case.

A same-material multi-UV-island ownership control is available through:

```powershell
just render-parity-uv-island-ownership-generated
just render-parity-uv-island-ownership-owner-generated
just render-parity-uv-island-ownership-owner-hotspots
```

It writes `.external-fixtures/generated/uv-island-ownership.vrm.gltf` with
three overlapping `huku_bake` mesh nodes that share one MToon material but
sample separated high-gradient regions of one generated base texture. This is a
source-like control for the Seed-san class where local material names match but
nearby UV islands can produce very different base colors. The 2026-06-19
base-color run under
`.external-fixtures/render-parity-uv-island-ownership-generated/` passes exact
alpha parity with selected `rgb-shared-nonblack-interior1px` PSNR wgpu
`57.3121 dB` / Bevy `51.2217 dB` and max selected-channel delta `3`. The top
`32` hotspot summaries stay on `huku_bake`, match the frontmost material for
both expected and actual pixels, and have actual colors closer than three-vrm to
the CPU-sampled frontmost base texture for `32/32` hotspots while exercising
large local texture gradients (`>=96` for all `32/32`). The owner-id recipe
keeps wgpu and Bevy exactly identical; each Rust renderer differs from three-vrm
by only `8/28554` shared nonzero owner pixels, all explained by adjacent or
touching projected triangles with zero unexplained tail. The owner-hotspot
projection has `32/32` rendered owners already at frontmost rank 1 at the center
sample. This narrows the remaining Seed-san blocker away from generic
same-material UV-island/high-gradient sampling and toward more specific
real-model topology, draw grouping, or browser fill behavior around dense
material regions.

An additional `rgb-shared-nonblack-interior2px` Seed-san base-UV run writes
`.external-fixtures/render-parity-seed-base-uv-interior2-diagnostic/` and
reports wgpu `39.1371 dB` / Bevy `38.9656 dB`, still with max selected-channel
delta `166`. Dropping a two-pixel shared-nonblack boundary band improves the
score only modestly, so the residual is not just the outer silhouette; it is a
thin but real set of internal UV seam / triangle-boundary selections.

The outline-isolated variant is:

```powershell
just render-parity-seed-base-uv-outline-off-diagnostic
```

It writes
`.external-fixtures/render-parity-seed-base-uv-outline-off-diagnostic/`, including
direct-imqraw reports, hotspot delta reports, and hotspot-to-primitive maps with
`--disable-outlines`. On the 2026-06-10 Seed-san run, fixing Rust diagnostic
outline handling makes the normal base-UV diagnostic and this outline-off
variant agree at wgpu `38.6925 dB` / Bevy `38.5360 dB` on
`rgb-shared-nonblack-interior1px`. Treat the remaining hotspots as base surface
UV/lookup-locality work rather than outline expansion work.

The local runner also verifies every renderer's direct `.imqraw` artifact
against its companion `.rgba.json` artifact before writing PNGs or comparing
three-vrm/wgpu/Bevy. For a focused check, use:

```powershell
cargo +nightly -Zscript tools/render-parity/verify-imqraw-rgba.rs `
  --imqraw .external-fixtures/render-parity/wgpu/Seed-san.frame000.imqraw `
  --rgba-json .external-fixtures/render-parity/wgpu/Seed-san.frame000.rgba.json
```

The `just imqraw-verify IMQRAW RGBA_JSON` wrapper runs the same check.

For independent raw-image metric checks that do not pass through PNG encoding
or decoding, use the `imqraw` TypeScript/WASM pack path:

```powershell
just imqraw-compare-rgba `
  .external-fixtures/render-parity-real-normal-maps/three-vrm/Seed-san.frame000.rgba.json `
  .external-fixtures/render-parity-real-normal-maps/wgpu/Seed-san.frame000.rgba.json `
  .external-fixtures/render-parity-real-normal-maps/reports/Seed-san.wgpu-vs-three-vrm.imqraw-ts.json
```

This runs `tools/render-parity/imqraw-compare-rgba-json.ts`, imports the fixed
`https://sanzentyo.github.io/imq/imqraw/v0.1.0/imqraw.js` distribution, packs
the two `.rgba.json` buffers with `encodeBundle`, and pipes the resulting
lossless `imqraw` bytes to `imq image - - --stdin-format imqraw`. The PNG and
HTML artifacts remain for visual review only; this path compares the raw RGBA
buffers produced by the renderers. The remaining feature gap before replacing
the repository-local comparator with the public `imq` CLI is tracked in
`docs/imq-compare-vrm-parity-requirements.md`.

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

The report contains dimensions, MSE, MAE, PSNR, maximum channel delta, maximum
pixel delta, alpha counts/mismatches, RGB-only opaque/visible/interior metrics, the
selected metric, and pass/fail status. Exact matches report `"Infinity"` for
PSNR. The comparator accepts `--metric rgba`, `--metric rgb-opaque`,
`--metric rgb-visible`, `--metric rgb-nonblack`,
`--metric rgb-interior1px`, `--metric rgb-visible-interior1px`,
`--metric rgb-nonblack-interior1px`, and
`--metric rgb-shared-nonblack-interior1px`, and
`--metric rgb-shared-nonblack-interior2px`, and
`--metric rgb-shared-nonblack-interior3px`; pass/fail thresholds use the selected
metric. The nonblack metrics are intended for opaque-black review
sweeps where empty background pixels should not dilute the model-body color
error. The shared-nonblack metric is stricter for focused diagnostics: a pixel
is included only when both compared images and their one-pixel neighbors are
nonblack, so it isolates overlapping model-body color from silhouette
classification differences. It also
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
still dropping one-pixel silhouette edges, and use
`rgb-shared-nonblack-interior1px` when both sides must draw nonblack content
before a body-color pixel is admitted. Use
`rgb-shared-nonblack-interior2px` as a stricter diagnostic when a two-pixel
body mask is needed to distinguish edge contamination from interior texture
residuals. Use `rgb-shared-nonblack-interior3px` only as a sharper local
diagnostic for persistent shared-edge/raster-ownership residuals, not as the
default whole-sample acceptance metric.

For a full real-sample shaded sweep that uses that stricter shared-body mask,
run:

```powershell
just render-parity-samples-shared-body3
```

This writes `.external-fixtures/render-parity-samples-shared-body3/`, selects
`rgb-shared-nonblack-interior3px`, and enforces `--render-fail-under 28.5`.
The current six-fixture floor is the constraint sample at wgpu `28.6412 dB` /
Bevy `28.6300 dB`; Seed-san is wgpu `31.9233 dB` / Bevy `31.2229 dB`, UV
animation is wgpu `30.1907 dB` / Bevy `30.1538 dB`, the expression mask samples
are wgpu `51.6179 dB` / Bevy `50.1671 dB`, and Alicia VRM0 is wgpu
`33.1953 dB` / Bevy `33.1425 dB`. This is not a replacement for the canonical
`rgb-visible` review; it is the body-color/material sweep used after known
local raster ownership bands have been classified separately.

The current shared-body floor is outline-sensitive rather than normal-map
sensitive. `just render-parity-constraint-shared-body3-diagnostics` reruns the
constraint sample with either outlines or normal maps disabled. With normal maps
disabled, the shared-body score remains essentially flat at wgpu `28.6428 dB` /
Bevy `28.6321 dB`. With outlines disabled, it rises to wgpu `31.0592 dB` /
Bevy `31.0407 dB`. The top-64 shared-body hotspots are identical between wgpu
and Bevy, mostly on materials `12`, `6`, `7`, `9`, and `3`, with large
base/outline color swaps near internal edges. This makes real outline
expansion/color ownership the next constraint-sample parity target.

`just render-parity-constraint-shared-body3-pass-summary` maps the same top-64
hotspots with expanded outline geometry and base/outline pass ownership counts.
The current result is identical for wgpu and Bevy at the pass level:
frontmost visible candidates are `57` base / `7` outline, nearest actual-color
candidates are `58` base / `6` outline, and nearest expected-color candidates
are `61` base / `3` outline. Pass matches are `51/64` for actual-vs-frontmost
and `56/64` for expected-vs-frontmost, while `62-63/64` hotspots are still
within `0.25px` of the frontmost nearest edge. A trial that inverted outline
lighting normals for normal-map-disabled outline passes slightly worsened the
constraint score, so that branch is kept out of the renderer path; the remaining
work is edge/pass ownership and material color selection, not a global outline
normal sign change.

The hotspot mapper also records frontmost-to-nearest surface transitions. For
the constraint top-64 run, the dominant transitions are base-material seams, not
outline ownership: `material_6 -> material_12` (`13` actual / `12` expected),
`material_12 -> material_4` (`6` / `6`), `material_12 -> material_7` (`4` /
`6`), `material_9 -> material_12` (`4` / `5`), and `material_12 -> material_3`
(`4` / `5`). The only top transition involving outline is
`outline material_0 -> base material_0` at `3` pixels. The next useful
constraint-sample work is therefore material seam/fill-rule classification
around those base-material boundaries before more shader tuning.
`just render-parity-material-seam-generated` now isolates that branch on a
license-safe generated glTF. Its remaining deltas are only seven base-pass seam
pixels, so broad changes to material assignment, culling, or draw ordering
should preserve this guard before being evaluated on the real constraint sample.

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

- `.external-fixtures/render-parity/three-vrm/<fixture>.frame000.{rgba.json,png,imqraw}`
- `.external-fixtures/render-parity/wgpu/<fixture>.frame000.{rgba.json,png,imqraw}`
- `.external-fixtures/render-parity/bevy/<fixture>.frame000.{rgba.json,png,imqraw}`
- `.external-fixtures/render-parity/reports/<fixture>.{wgpu,bevy}-vs-three-vrm.imqraw-rust.json` (numeric gate)
- `.external-fixtures/render-parity/reports/<fixture>.{wgpu,bevy}-vs-three-vrm.psnr.json` (RGBA JSON diagnostic)
- `.external-fixtures/render-parity/diff/<fixture>.{wgpu,bevy}-vs-three-vrm.diff.png`
- `.external-fixtures/render-parity/summary.md`
- `.external-fixtures/render-parity/visual-review.html`
- `.external-fixtures/render-parity/review-manifest.json`

`summary.md` is the compact audit artifact for PSNR and alpha review. It lists
the selected metric, background, MToon light accumulation mode, per-fixture
wgpu/Bevy selected PSNR, max selected-channel delta, alpha mismatch count,
alpha max delta, and pass/fail status. `visual-review.html` embeds the same
summary before the side-by-side PNGs and diff heatmaps.
`review-manifest.json` is the machine-readable audit index: it links each
fixture and renderer to the source VRM, reference/capture RGBA JSON, direct
imqraw, preview PNG, numeric gate report, RGBA diagnostic report, diff heatmap,
and pass/fail summary.
The local runner validates this manifest before completing. To re-check an
existing artifact set, run:

```powershell
cargo +nightly -Zscript tools/render-parity/validate-review-manifest.rs `
  --manifest .external-fixtures/render-parity/review-manifest.json
```

The `just render-parity-validate MANIFEST` wrapper runs the same audit. The
validator also cross-checks the manifest's reference/capture artifact paths
against the `expected` and `actual` fields embedded in both the direct-imqraw
numeric report and RGBA diagnostic report, and requires the manifest summary
strings to match the numeric report's selected PSNR, selected max-channel
delta, alpha mismatch count, and alpha max delta.

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
compositing. Since the numeric gate now reads `.imqraw`, the runner first
checks that each `.imqraw` artifact contains the same RGBA8 pixels as the
`.rgba.json` artifact used for PNG, diff, and diagnostic output. At the start of
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
`--render-fail-under 34`. Current selected PSNR is Seed-san wgpu `34.6181 dB` /
Bevy `34.0835 dB`, and constraint sample wgpu `36.2443 dB` / Bevy
`36.2352 dB`, all with alpha mismatches `0`. The Bevy capture fills missing
generated tangent vertices with `vrm-io::fallback_tangent`, so one degenerate
vertex no longer disables normal maps for the whole primitive. Review
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

There is also a view-space derivative diagnostic:

```powershell
just render-parity-normal-maps-view-derivative
```

This uses a view-space derivative tangent frame and transforms the perturbed
normal back into the Rust captures' world-space lighting path. It is closer in
shape to three-vrm's tangentless WebGL shader than the older world-space
derivative mode, but the first 64px Seed-san smoke on 2026-06-10 still measured
below the default generated-tangent path at wgpu `29.5312 dB` / Bevy
`29.5376 dB`; keep it diagnostic until a follow-up change improves the result.

For normal-map strength diagnostics, pass `--render-normal-map-scale N` to the
local runner. This scales only the Rust wgpu/Bevy capture normal-map strength;
the three-vrm reference remains native. Keep the default at `1.0` for parity
runs. On the 2026-06-10 Seed-san diagnostic, both `0.75` and `1.25` were worse
than the default, so the current blocker points more toward tangent-frame,
normal texture sampling, or rasterization details than a simple normal-strength
coefficient.

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
fallback behavior. Tangent generation now goes through the shared
`vrm-io::generate_tangents` helper used by both wgpu and Bevy. The current run
has exact alpha buckets and mismatches `0`; selected `rgb-interior1px` PSNR is
wgpu `47.8977 dB` with max selected channel delta `11`, and Bevy `46.7852 dB`
with max selected channel delta `11`. The Bevy path keeps generated tangent
frames when a primitive accessor includes unreferenced vertices and enables
`VERTEX_TANGENTS` for the custom material shader. The recipe now enforces
`rgb-interior1px >= 46.5 dB`. Treat this as the current normal-map regression
guard, not final visual parity; the remaining work is to confirm real
tangentless official primitives with higher thresholds.

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
shared box downsampling for the current capture path. Renderer-facing glTF
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
`GltfMaterialTextureSlots::binding_plan` further shares the shader-slot
binding contract: each slot carries its resolved texture index, sRGB versus
linear upload-table choice, and white/black/neutral-normal fallback before a
renderer creates handles, bind groups, or descriptor sets.
The matching `LoadedVrm::material_uv_transforms` helper now centralizes MToon
texture transforms, glTF texture-transform fallbacks, shade fallback-to-base
behavior, and time-based UV animation scroll/rotation before the concrete
captures apply expression-driven transform overrides.
`GltfMaterialUvTransforms::uniform_plan` then packs those transforms into the
shared shader-facing offset/scale and rotation arrays, keeping texCoord0 policy
and UV animation uniform layout identical for wgpu, Bevy, and future adapters.
`LoadedVrm::material_shading_plan` similarly centralizes the renderer-facing
MToon/PBR fallback shading inputs, including effective emissive strength,
normal scale, rim/matcap parameters, and VRM0 compatibility flags, while
`LoadedVrm::expression_render_effects` centralizes the expression-driven morph,
material-color, and texture-transform overrides that are applied on top of that
base material plan before each capture builds backend resources.
`vrm-adapter::renderer_material_pipeline_plan` now centralizes the MToon base
pass pipeline policy plus glTF alpha/double-sided override merge, so wgpu,
Bevy, ash-style examples, and custom renderers can select cull/depth/blend
state without reimplementing capture-only policy.
`GltfPrimitiveData::morphed_vertex` centralizes the local morph target
accumulation used by wgpu and Bevy before skinning, outline expansion, generated
tangents, and backend mesh construction.
`GltfSkinData::joint_matrices`, `vrm-io::skin_vertex`, and
`vrm-io::skin_direction` now centralize CPU-side joint matrix assembly plus
position/normal/tangent-direction skinning, removing another duplicate
renderer-edge path before wgpu, Bevy, ash-style examples, or custom engines map
the data into backend meshes.
`vrm-io::generate_tangents` likewise centralizes tangentless normal-map tangent
generation, including unreferenced-vertex fallback behavior and per-vertex
failure reporting, before concrete backends decide how to expose tangent frames
to their shaders.
CPU-side outline-width texture sampling also shares
`vrm-io::transform_tex_coord_0` for offset/scale/rotation application on UV set
0, keeping the wgpu and Bevy diagnostic path aligned.
MToon light accumulator resolution is now shared through
`vrm-adapter::MtoonLightingConfig`, so the tuned diagnostic mode and the
reference-shaped `three-vrm` mode feed identical effective lighting values into
wgpu and Bevy before each backend performs shader-specific material work.

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
mismatches `0`. The capture paths now use shared box mip-chain downsampling for
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
That expansion math is now exposed by `vrm-io` as
`GltfPrimitiveData::outline_position` plus
`GltfOutlineScale`/`GltfOutlineSettings`, so wgpu, Bevy, ash-style examples,
and custom engines can reuse the same Sans I/O morph/skin-aware outline
position calculation before converting vertices into backend buffers.
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
