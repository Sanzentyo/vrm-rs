# Current Rendering Image Comparison

Updated: 2026-06-22

This page is a compact visual board for the render-parity artifacts that exist in this workspace right now. The images and raw comparison files live under `.external-fixtures/` and are intentionally not committed. Use [render-parity-image-comparison.md](render-parity-image-comparison.md) for the broader historical index.

For a shorter side-by-side board focused only on the currently generated images,
use [rendering-result-current-comparison.md](rendering-result-current-comparison.md).
For a Japanese review board using the same current image artifacts, use
[rendering-result-current-comparison.ja.md](rendering-result-current-comparison.ja.md).

Reference images are rendered with `three-vrm`. Compared images are Rust renderers: `wgpu`, Bevy, and Ash. Numeric values below are read from the current `.imqraw` comparison reports.

## Current Artifact Sets

| Purpose | Directory | Main comparison |
| --- | --- | --- |
| Current base-UV rerun | [`.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun) | `rgb-visible` |
| Opaque real sample sweep | [`.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check) | `rgb-visible` |
| Transparent real sample sweep | [`.external-fixtures/render-parity-real-transparent-ash-gated`](../.external-fixtures/render-parity-real-transparent-ash-gated) | `rgb-all` |
| Generated glTF/PBR fallback | [`.external-fixtures/render-parity-gltf-pbr-generated`](../.external-fixtures/render-parity-gltf-pbr-generated) | `rgb-interior1px` |
| Generated transparent blend | [`.external-fixtures/render-parity-transparent-generated-ash-gated`](../.external-fixtures/render-parity-transparent-generated-ash-gated) | `rgb-visible` |
| Current Seed-san base-color diagnostic | [`.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback) | `rgb-shared-nonblack-gradient-interior1px` |
| Expanded post-resolve diagnostic | [`.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback) | focused owner/sample check |
| Rejected second-frontier diagnostic | [`.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback) | negative-control only |
| Owner/base-color hotspot join | [`.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/reports/Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/reports/Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md) | source/owner diagnosis |

## Quick Metrics

### Current Base-UV Rerun

This is the current image set to open first when visually comparing the latest
three-vrm, wgpu, Bevy, and Ash outputs. The capture uses `base-uv` diagnostic
rendering, opaque black background, direct `.imqraw` reports, and exact alpha
parity across all three Rust renderers.

| Renderer | `rgb-visible` PSNR | `rgbSharedNonblackGradientInterior1px` PSNR | Changed RGB pixels | Max channel delta | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: | ---: |
| wgpu | 36.8913 | 32.6698 | 1107 | 251 | 0 |
| Bevy | 36.8708 | 32.6368 | 9252 | 251 | 0 |
| Ash | 36.8913 | 32.6698 | 1107 | 251 | 0 |

The visual gate passes for this diagnostic set, but the high max-channel delta
and the lower gradient-domain metric show that the remaining work is still
localized material/color and edge ownership parity rather than alpha or raw
readback format mismatches.

### Opaque Real Samples

| Fixture | wgpu | Bevy | Ash | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | wgpu 0 / Bevy 0 / Ash 0 |
| `VRM1_Constraint_Twist_Sample.vrm` | 36.2518 | 36.2349 | 36.2509 | wgpu 0 / Bevy 0 / Ash 0 |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | wgpu 0 / Bevy 0 / Ash 0 |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | 55.6968 | 55.2106 | 55.6968 | wgpu 0 / Bevy 0 / Ash 9 |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | 55.7181 | 55.2306 | 55.7181 | wgpu 0 / Bevy 0 / Ash 9 |
| `AliciaSolid_vrm-0.51.vrm` | 35.6238 | 35.6088 | 35.6238 | wgpu 0 / Bevy 0 / Ash 0 |

### Focused Generated Guards

| Fixture | Metric | wgpu | Bevy | Ash | Max channel delta |
| --- | --- | ---: | ---: | ---: | ---: |
| `gltf-pbr_vrm` | `rgb-interior1px` | 47.8937 | 47.4289 | 47.8937 | 6 |
| `transparent-blend_vrm` | `rgb-visible` | 54.3997 | 56.8605 | 54.3997 | 1 |

### Current Seed-san Base-Color Diagnostic

This is the current root visual blocker. Alpha matches exactly. The current
readback still diverges at representative material pixels, while the expanded
post-resolve diagnostic proves that those same pixels can be written back to the
three-vrm expected values when the correct source sample is forced. The
second-frontier run is kept only as a negative control because it changes the
source-derived selection frontier without explaining the renderer behavior.

| Set | wgpu gradient PSNR | Bevy gradient PSNR | Ash gradient PSNR | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: |
| Current readback | 30.6336 | 28.2142 | 35.8902 | 0 / 0 / 0 |
| Expanded post-resolve diagnostic, current local artifact | 32.7302 | 32.4826 | 35.8901 | 0 / 0 / 0 |
| Second-frontier negative control | 30.9642 | 28.6200 | 35.8901 | 0 / 0 / 0 |

Focused wgpu material-pixel reports:

- Current readback:
  [`Seed-san.wgpu-focused-material-pixels.gradient.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.wgpu-focused-material-pixels.gradient.md)
- Expanded post-resolve:
  [`Seed-san.wgpu-focused-material-pixels.render-resolve-expanded.gradient.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.wgpu-focused-material-pixels.render-resolve-expanded.gradient.md)

Owner/sample execution audits compare each post-resolve manifest entry's target
pixel against the actual readback pixel. They do not choose or modify samples.
The important split is not just whether the readback equals the manifest sample,
but whether the readback or the manifest sample is closer to the three-vrm
expected pixel. After the Bevy UV0/UV1 gradient update, Bevy no longer simply
follows the manifest sample: the expanded readback now lands near the
three-vrm expected pixel for most entries, much like the wgpu/Ash split.

| Readback | Renderer | Entries | Actual~sample | Actual closer to expected | Sample closer to expected | Tie | Mean actual-sample | Mean actual-expected |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Current | wgpu | 101 | 10 | 53 | 43 | 5 | 71.6334 | 43.7365 |
| Current | Bevy | 93 | 58 | 24 | 44 | 25 | 33.4403 | 65.0680 |
| Current | Ash | 99 | 10 | 76 | 18 | 5 | 55.6954 | 25.6181 |
| Expanded | wgpu | 101 | 24 | 77 | 18 | 6 | 45.7751 | 20.3921 |
| Expanded | Bevy | 93 | 20 | 65 | 23 | 5 | 44.7858 | 20.1580 |
| Expanded | Ash | 99 | 10 | 76 | 18 | 5 | 55.6968 | 25.6175 |

Execution audit reports:

- Current wgpu:
  [`Seed-san.wgpu-owner-sample-execution.current-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.wgpu-owner-sample-execution.current-readback.md)
- Current Bevy:
  [`Seed-san.bevy-owner-sample-execution.current-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.bevy-owner-sample-execution.current-readback.md)
- Current Ash:
  [`Seed-san.ash-owner-sample-execution.current-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.ash-owner-sample-execution.current-readback.md)
- Expanded wgpu:
  [`Seed-san.wgpu-owner-sample-execution.expanded-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.wgpu-owner-sample-execution.expanded-readback.md)
- Expanded Bevy:
  [`Seed-san.bevy-owner-sample-execution.expanded-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.bevy-owner-sample-execution.expanded-readback.md)
- Expanded Ash:
  [`Seed-san.ash-owner-sample-execution.expanded-readback.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.ash-owner-sample-execution.expanded-readback.md)

Each execution audit now includes a `Top Sample-Closer Expected Pixels` table.
That table isolates the pixels where the manifest sample is closer to
`three-vrm` than the current renderer output. In the current local artifacts,
the largest wgpu/Ash sample-closer buckets are concentrated around
`armgear_plastic`, `body_nm`, `arm_mat`, `arm_plastic`, `backpack_nm`, and
`huku_bake`. Bevy's expanded readback now behaves more like a material-evaluated
resolve than a sample-copy path, so the same table should be read as a remaining
targeted owner/sample miss list rather than proof that Bevy should preserve the
old manifest-sample output.

The audits also include `Sample-Closer Buckets By Material` and
`Sample-Closer Buckets By Material And Source`. They now also include draw-key
views (`node/mesh/primitive/pass`) sourced from manifest `sample_geometry`. On
the expanded readback, the largest actionable wgpu/Ash margins are not the same as the largest counts:
`huku_bake` has the most sample-closer rows but low mean margin, while `body_nm`,
`arm_plastic`, `arm_mat`, and `armgear_plastic` have much larger mean margins.
`backpack_nm` appears as only one low-margin sample-closer row in this audit, so
the remaining `backpack_nm` work should stay in the separate material/color
evaluation bucket rather than being folded into the owner/sample miss bucket.

| Expanded readback | Top sample-closer count bucket | Highest-margin buckets | Reading |
| --- | --- | --- | --- |
| wgpu | `huku_bake` `6` rows, mean margin `3.9800` | `arm_plastic` `46.7300`, `body_nm` `43.7600`, `arm_mat` `40.9900` | Owner/sample miss rows exist, but the biggest errors are body/plastic/arm surfaces. |
| Ash | `huku_bake` `5` rows, mean margin `4.5800` | `arm_plastic` `46.7300`, `body_nm` `43.7600`, `arm_mat` `40.9900` | Same priority shape as wgpu; backend transport is not the only issue. |
| Bevy | `huku_bake` `9` rows, mean margin `3.1171` | `arm_plastic` `46.4147`, `body_nm` `42.6551`, `arm_mat` `39.7288` | Bevy is no longer mostly sample-following; remaining sample-closer rows overlap the same body/plastic/arm buckets. |

| High-margin material | Draw key | wgpu rows / mean margin | Ash rows / mean margin | Bevy rows / mean margin |
| --- | --- | ---: | ---: | ---: |
| `body_nm` | `node145/mesh4/prim1/base` | 4 / 43.7600 | 4 / 43.7600 | 1 / 0.3200 |
| `arm_plastic` | `node144/mesh3/prim1/base` | 2 / 46.7300 | 2 / 46.7300 | 3 / 1.0300 |
| `arm_mat` | `node144/mesh3/prim0/base` | 2 / 40.9900 | 2 / 40.9900 | 0 / n/a |
| `armgear_plastic` | `node145/mesh4/prim4/base` | 2 / 37.8500 | 3 / 30.0900 | 3 / 0.7200 |

Renderer RGBA JSON artifacts now expose the same grouping under
`renderer.ownerSampleCorrectionPlan.drawSelections`, so the next parity pass can
verify whether each high-margin manifest entry reached its intended draw before
looking at final pixel color. A wgpu smoke against the expanded post-resolve
manifest reported `21` draw selections with nonzero entries for all four
high-margin draw keys above.

The current local expanded Seed-san artifacts report gradient PSNR wgpu
`32.7302 dB`, Bevy `32.4826 dB`, and Ash `35.8901 dB` with exact alpha. These
numbers come from the full `render-resolve-expanded-readback` recipe after
regenerating the source-derived manifests and texture audits, so they supersede
isolated focused Bevy reruns. This diagnostic remains useful for target-pixel
coverage and routed-sample analysis, but it is not a full default behavior
target: the refreshed texture audit still reports selected mean E-A distance
wgpu `45.8852`, Bevy `45.3511`, and Ash `36.9738`.

The current generated summary is:
[`Seed-san.render-resolve-expanded.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.md)

That generated summary now also embeds the shading-model residual join. Use its
`Shading Model Backend Agreement` section as the quickest current split between
`gltf_pbr` backpack color accumulation and MToon eye/body/plastic/bake residuals
before drilling into the larger `target/texture-draw-audit` reports.

The E-A direction splits by material/draw key. For example, wgpu
`backpack_nm node145/mesh4/prim9/base` is expected-brighter
(`+18.47,+21.00,+22.33`), while `body_nm node145/mesh4/prim1/base` is
expected-darker (`-33.00,-26.50,-24.75`); Ash shows the same directional split
at smaller magnitude. Use these audit rows to split glTF/PBR backpack color
accumulation from MToon body/plastic local material differences, rather than
adding another global exposure or color-space toggle.

The same expected-vs-actual audit now also includes frontmost shading-model
buckets. This makes the split visible before drilling into material names:
wgpu selected residuals separate into `mtoon` (`24` rows, mean E-A `52.18`) and
`gltf_pbr` (`15` rows, mean E-A `35.81`), while Ash separates into `mtoon`
(`41`, `39.77`) and `gltf_pbr` (`23`, `31.99`). Treat `gltf_pbr` as the
backpack color-accumulation track and `mtoon` as the body/plastic local
material/fill track.

The audit also emits `top_residuals_by_shading_model` in JSON and a matching
Markdown section. Use those rows as stable pixel probes when changing renderer
material code: `gltf_pbr` rows isolate the backpack/PBR path, while `mtoon`
rows keep the body, arm, plastic, and bake surfaces from being mixed into the
same evidence bucket.

`just render-parity-seed-base-color-flat32-shading-model-residual-join` now
joins those model-specific probes across wgpu, Bevy, and Ash. The current join
report shows `gltf_pbr` has `16` shared top-residual pixels and is entirely
`backpack_nm node145/mesh4/prim9/base`; all three backends are now in the same
mean E-A band (`33.45` wgpu, `33.22` Bevy, `32.65` Ash). `mtoon` also has `16`
shared top-residual pixels, with wgpu `45.87`, Bevy `45.89`, and Ash `45.62`.

The join report also emits backend color-fit, shared backend sample-following,
backend-pair agreement, and direction buckets. On the current expanded Seed-san
artifacts, `gltf_pbr` still shows Ash/wgpu actual RGB distance `2.92`, but
Bevy/wgpu is even closer at `0.99`; `mtoon` is tightly clustered across all
pairs (`0.14` Ash/wgpu, `0.79` Bevy/wgpu). Every backend/model currently
prefers an additive RGB fit over a global gain fit. This points the next parity
work toward ambient/fill/light accumulation or material-local offsets, not a
single backend-wide exposure/gain correction or a Bevy-only selected-sample
story.

The same join now reports `Material / Draw Color Fit` under each shading model.
For `gltf_pbr`, all three backends isolate the same `backpack_nm
node145/mesh4/prim9/base` draw as additive-dominant: Ash `+17.06,+19.12,+20.19`
with error `5.48`, Bevy `+18.23,+20.46,+21.69` with error `3.98`, and wgpu
`+18.50,+20.83,+22.25` with error `3.78`. For `mtoon`, the `eye
node2/mesh2/prim1/base` buckets are also additive-dominant, while
`arm_mat node144/mesh3/prim0/base` is close enough that Bevy/wgpu prefer gain.
That makes the next implementation split concrete: debug backpack/PBR fill or
ambient as one track, and MToon eye/arm material accumulation as another.

The expanded summary JSON keeps those additive/gain `color_fit` values as
machine-readable data. If an upstream join row contains `color_fit: null` plus a
populated `color_fit_summary`, the parser treats the null as absent and uses the
populated fit block. Renderer material/draw artifacts now also expose explicit
`materialExtra.shaderBranch` values (`gltf_pbr`, `mtoon`, or `unlit`), so use
`branch:*` rows instead of older compact `pbr:*` text when checking current
shader-path parity.

Expected-vs-actual audit Markdown:

- [`Seed-san.wgpu.expected-actual.md`](../target/texture-draw-audit/Seed-san.wgpu.expected-actual.md)
- [`Seed-san.bevy.expected-actual.md`](../target/texture-draw-audit/Seed-san.bevy.expected-actual.md)
- [`Seed-san.ash.expected-actual.md`](../target/texture-draw-audit/Seed-san.ash.expected-actual.md)
- [`Seed-san.shading-model-residual-join.md`](../target/texture-draw-audit/Seed-san.shading-model-residual-join.md)

`audit-owner-sample-execution.rs` now reads that artifact metadata and adds a
`Renderer Draw Selection Routing` section. The same focused wgpu smoke routed
`101/101` manifest entries into draw selections with `0` missing entries, so
the current high-margin residuals are not explained by manifest rows being
dropped before draw assignment. The next root-cause work should inspect resolve
shading/writeback for those routed draws, especially `node144/mesh3/prim0/base`,
`node144/mesh3/prim1/base`, `node145/mesh4/prim1/base`, and
`node145/mesh4/prim4/base`.

The capture examples also have an `owner-sample-resolve` diagnostic render mode
for this handoff. It paints only owner/sample resolve writes green and the rest
black. Local focused smokes against the expanded manifests reported exact target
coverage with no extra green pixels: wgpu `101/101` manifest hits and `101`
green pixels total, Bevy `93/93` manifest hits and `93` green pixels total.
That leaves the current visible residual in material sampling / color modelling,
not in target-pixel write coverage.

## Current Base-UV Images

These are the latest local images under
`.external-fixtures/render-parity-ash-current-base-uv-rerun`. Use this section
for quick visual review, then use the linked raw reports for numeric decisions.

| three-vrm reference | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/three-vrm/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/ash/Seed-san.frame000.png" width="180"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.ash-vs-three-vrm.diff.png" width="180"> |

Raw reports:

- [`Seed-san.wgpu-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.wgpu-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.bevy-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.bevy-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.ash-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.ash-vs-three-vrm.imqraw-rust.json)

Focused material-pixel report for the current Seed-san blocker:

- [`Seed-san.wgpu-focused-material-pixels.gradient.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.wgpu-focused-material-pixels.gradient.md)

## Seed-san Current Diagnostic Images

The `three-vrm` reference image for this diagnostic comes from the base-color outline-off capture, while the Rust images below are the current render-resolve readbacks. Ash PNGs for these focused diagnostics are generated from the raw `.rgba.json` artifacts with `just render-parity-current-ash-pngs`.

| three-vrm reference | wgpu readback | Bevy readback | Ash readback |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/ash/Seed-san.frame000.png" width="180"> |

### Expanded Post-Resolve Diagnostic Images

This is not the desired default behavior. It is a diagnostic comparison that
uses the post-resolve source manifest to verify whether the renderer can express
the expected colors once the source ownership decision is supplied. The focused
report above shows 0.0000 actual-vs-expected RGB distance for the five selected
wgpu pixels.

| three-vrm reference | wgpu expanded readback | Bevy expanded readback | Ash expanded readback |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.png" width="180"> |

### Current Hotspot Reading

The latest owner/base-color join uses 64 current hotspots from the `three-vrm` readback and joins the owner-id projection with the base-color projection. It keeps the rendered image comparison grounded in the same pixels instead of guessing from global PSNR alone.

| Joined hotspots | Owner/frontmost material matches | Owner/frontmost surface matches | Mean owner-surface base distance | Mean browser-compatible distance | Frontmost-to-nearest draw order |
| ---: | ---: | ---: | ---: | ---: | --- |
| 64 | 56 | 27 | 86.2706 | 62.5942 | same 24 / after 23 / before 17 |

The important current reading is that most hotspots keep the same material owner, but only 27/64 keep the same surface. The browser-compatible base-color projection is closer on some buckets, especially `backpack_nm`, but worse on others, so it remains a diagnostic axis rather than a default color-space behavior change. Older artifacts may still carry this value under `texture_as_linear` field names. The same-stream draw-order split also rules out a simple "later draw always wins" explanation for the current base-color residual. Use the joined report for the detailed per-pixel material and draw-order transitions:

[`Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/reports/Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md)

### Negative Control: Second Frontier

These images are useful for review, but they are not a desired default behavior. They show why repeated owner-frontier expansion should stay diagnostic until the source behavior is explained.

| wgpu second-frontier | Bevy second-frontier | Ash second-frontier |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/ash/Seed-san.frame000.png" width="180"> |

## Real Sample Visual Sweep

### Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/Seed-san.frame000.png" width="180"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.ash-vs-three-vrm.diff.png" width="180"> |

### Constraint Sample

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRM1_Constraint_Twist_Sample.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRM1_Constraint_Twist_Sample.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRM1_Constraint_Twist_Sample.frame000.png" width="180"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.wgpu-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.bevy-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.ash-vs-three-vrm.diff.png" width="180"> |

### UV Animation

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="180"> |

### VRM0 AliciaSolid

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/AliciaSolid_vrm-0_51.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/AliciaSolid_vrm-0_51.frame000.png" width="180"> |

## Transparent And PBR Guards

### Generated Transparent Blend

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/three-vrm/transparent-blend_vrm.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/wgpu/transparent-blend_vrm.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/bevy/transparent-blend_vrm.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/ash/transparent-blend_vrm.frame000.png" width="180"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.wgpu-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.bevy-vs-three-vrm.diff.png" width="180"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.ash-vs-three-vrm.diff.png" width="180"> |

### Generated glTF/PBR Fallback

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/three-vrm/gltf-pbr_vrm.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/wgpu/gltf-pbr_vrm.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/bevy/gltf-pbr_vrm.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/ash/gltf-pbr_vrm.frame000.png" width="220"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.wgpu-vs-three-vrm.diff.png" width="220"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.bevy-vs-three-vrm.diff.png" width="220"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.ash-vs-three-vrm.diff.png" width="220"> |

## How To Refresh

Regenerate the main boards with:

```powershell
just render-parity-samples-ash-gated
just render-parity-real-transparent-ash-gated
just render-parity-gltf-pbr-generated
just render-parity-transparent-generated-ash-gated
just render-parity-seed-base-color-flat32-render-resolve-readback
```

For final parity judgement, prefer the `.imqraw` reports over PNG-only inspection. PNGs are useful for human review, but the raw files are the source of truth for PSNR, alpha, and max-channel-delta values.

For the current Seed-san material/color blocker, use the expanded texture audit reports under:

```text
.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.{wgpu,bevy,ash}-texture-sampling-audit.render-resolve-expanded.gradient.{json,md}
```

Those reports now include `LS gain A/M / E/M` and actual/expected-over-manifest RGB ratios for each recommended material probe, so visual review can be tied back to raw selected-sample response rather than PNG-only inspection or PSNR-driven sample selection.
