# Current Rendering Image Comparison

Updated: 2026-06-21

This page is a compact visual board for the render-parity artifacts that exist in this workspace right now. The images and raw comparison files live under `.external-fixtures/` and are intentionally not committed. Use [render-parity-image-comparison.md](render-parity-image-comparison.md) for the broader historical index.

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
| `gltf-pbr_vrm` | `rgb-interior1px` | 47.8016 | 47.2691 | 47.8016 | 6 |
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
| Expanded post-resolve diagnostic | 32.7302 | 29.2224 | 35.8901 | 0 / 0 / 0 |
| Second-frontier negative control | 30.9642 | 28.6200 | 35.8901 | 0 / 0 / 0 |

Focused wgpu material-pixel reports:

- Current readback:
  [`Seed-san.wgpu-focused-material-pixels.gradient.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.wgpu-focused-material-pixels.gradient.md)
- Expanded post-resolve:
  [`Seed-san.wgpu-focused-material-pixels.render-resolve-expanded.gradient.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.wgpu-focused-material-pixels.render-resolve-expanded.gradient.md)

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

The `three-vrm` reference image for this diagnostic comes from the base-color outline-off capture, while the Rust images below are the current render-resolve readbacks. Ash currently has raw `.rgba.json` / `.imqraw` artifacts in this directory, but no PNG image in the local artifact set.

| three-vrm reference | wgpu readback | Bevy readback |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/wgpu/Seed-san.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/bevy/Seed-san.frame000.png" width="220"> |

### Expanded Post-Resolve Diagnostic Images

This is not the desired default behavior. It is a diagnostic comparison that
uses the post-resolve source manifest to verify whether the renderer can express
the expected colors once the source ownership decision is supplied. The focused
report above shows 0.0000 actual-vs-expected RGB distance for the five selected
wgpu pixels.

| three-vrm reference | wgpu expanded readback | Bevy expanded readback |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.png" width="220"> |

### Current Hotspot Reading

The latest owner/base-color join uses 64 current hotspots from the `three-vrm` readback and joins the owner-id projection with the base-color projection. It keeps the rendered image comparison grounded in the same pixels instead of guessing from global PSNR alone.

| Joined hotspots | Owner/frontmost material matches | Owner/frontmost surface matches | Mean owner-surface base distance | Mean texture-as-linear distance | Frontmost-to-nearest draw order |
| ---: | ---: | ---: | ---: | ---: | --- |
| 64 | 56 | 27 | 86.2706 | 62.5942 | same 24 / after 23 / before 17 |

The important current reading is that most hotspots keep the same material owner, but only 27/64 keep the same surface. Texture-as-linear sampling is closer on some buckets, especially `backpack_nm`, but worse on others, so it remains a diagnostic axis rather than a default behavior change. The same-stream draw-order split also rules out a simple "later draw always wins" explanation for the current base-color residual. Use the joined report for the detailed per-pixel material and draw-order transitions:

[`Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/reports/Seed-san.owner-base-color-hotspots.gradient.0.5x0.5.summary.md)

### Negative Control: Second Frontier

These images are useful for review, but they are not a desired default behavior. They show why repeated owner-frontier expansion should stay diagnostic until the source behavior is explained.

| wgpu second-frontier | Bevy second-frontier |
| --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/wgpu/Seed-san.frame000.png" width="220"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/bevy/Seed-san.frame000.png" width="220"> |

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
