# Rendering Image Comparison

Updated: 2026-06-21

This is a local Markdown index for the render-parity images that exist in this workspace right now. The PNG, `.rgba.json`, and `.imqraw` artifacts stay under `.external-fixtures/` and are not source-controlled. Regenerate them with the listed `just` recipes before using this page on another machine.

The reference image is `three-vrm`. The compared images are Rust renderers: `wgpu`, `bevy`, and `ash`. Diff images are generated against `three-vrm`.

## Current Artifact Sets

| Set | Recipe | Artifact directory | Review page | Primary metric |
| --- | --- | --- | --- | --- |
| Real samples, opaque black | `just render-parity-samples-ash-gated` | [`.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check) | [`visual-review.html`](../.external-fixtures/render-parity-samples-ash-gated-check/visual-review.html) | `rgbVisible` |
| Real samples, transparent | `just render-parity-real-transparent-ash-gated` | [`.external-fixtures/render-parity-real-transparent-ash-gated`](../.external-fixtures/render-parity-real-transparent-ash-gated) | [`visual-review.html`](../.external-fixtures/render-parity-real-transparent-ash-gated/visual-review.html) | `rgbAll` / alpha mismatches |
| Generated transparent blend | `just render-parity-transparent-generated-ash-gated` | [`.external-fixtures/render-parity-transparent-generated-ash-gated`](../.external-fixtures/render-parity-transparent-generated-ash-gated) | [`visual-review.html`](../.external-fixtures/render-parity-transparent-generated-ash-gated/visual-review.html) | `rgbVisible` |
| Generated glTF PBR fallback | `just render-parity-gltf-pbr-generated` | [`.external-fixtures/render-parity-gltf-pbr-generated`](../.external-fixtures/render-parity-gltf-pbr-generated) | [`visual-review.html`](../.external-fixtures/render-parity-gltf-pbr-generated/visual-review.html) | `rgbInterior1px` |
| Current Seed-san base-color blocker | `just render-parity-seed-base-color-flat32-render-resolve-expanded2-readback` | [`.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback) | N/A | `selectedMetric` (`rgbSharedNonblackGradientInterior1px`) |

## Numeric Snapshot

### Real Samples, Opaque Black

Gate: exact alpha parity for wgpu/Bevy, Ash included in the visual gate, `rgbVisible >= 34 dB`.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Notes |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | Passes, but remains the main real-model owner/material-color target. |
| `VRM1_Constraint_Twist_Sample.vrm` | 36.2518 | 36.2349 | 36.2509 | Passes; residuals are local edge/material ownership. |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | Passes; UV animation parity is covered. |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | 55.6968 | 55.2106 | 55.6968 | Strong parity. |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | 55.7181 | 55.2306 | 55.7181 | Strong parity. |
| `AliciaSolid_vrm-0.51.vrm` | 35.6238 | 35.6088 | 35.6238 | Passes; VRM0 compatibility remains covered. |

### Generated glTF PBR Fallback

Gate: `rgbInterior1px >= 47 dB`, max selected channel delta `<= 6`, per-swatch `>= 40 dB`.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Notes |
| --- | ---: | ---: | ---: | --- |
| `gltf-pbr_vrm` | 47.8016 | 47.2691 | 47.8016 | Broad non-MToon glTF/PBR fallback, including normalTexture, is guarded. |

### Generated Transparent Blend

Gate: exact alpha parity and max channel delta `<= 1`.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: |
| `transparent-blend_vrm` | 54.3997 | 56.8605 | 54.3997 | 0 |

### Current Seed-san Base-Color Blocker

The expanded2 readback artifacts are diagnostic, not a default behavior target. The selected metric is `rgbSharedNonblackGradientInterior1px`; alpha is exact for all three renderers.

| Renderer | selectedMetric PSNR | selected max channel delta | Current reading |
| --- | ---: | ---: | --- |
| wgpu | 30.9642 | 149 | Blind second-frontier expansion regresses compared with the previous expanded readback. |
| Bevy | 28.6200 | 176 | Same issue as wgpu; the remaining gap is material/color/fill behavior, not just missing owner pixels. |
| Ash | 35.8901 | 59 | Better covered by the current source-derived selection, but still not full parity. |

## Real Sample Sweep

### Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/Seed-san.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.ash-vs-three-vrm.diff.png" width="160"> |

### VRM1 Constraint Twist Sample

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRM1_Constraint_Twist_Sample.ash-vs-three-vrm.diff.png" width="160"> |

### MToon UV Animation Test

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRMC_materials_mtoon_UV_Animation_Test.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRMC_materials_mtoon_UV_Animation_Test.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/VRMC_materials_mtoon_UV_Animation_Test.ash-vs-three-vrm.diff.png" width="160"> |

### Expression Samples

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Overridden | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> |
| Overrides | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> |

### AliciaSolid VRM0

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/AliciaSolid_vrm-0_51.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/AliciaSolid_vrm-0_51.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/AliciaSolid_vrm-0_51.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/AliciaSolid_vrm-0_51.ash-vs-three-vrm.diff.png" width="160"> |

## Transparent Spot Checks

### Real Transparent Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/three-vrm/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/wgpu/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/bevy/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/ash/Seed-san.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.ash-vs-three-vrm.diff.png" width="160"> |

### Generated Transparent Blend

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/three-vrm/transparent-blend_vrm.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/wgpu/transparent-blend_vrm.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/bevy/transparent-blend_vrm.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/ash/transparent-blend_vrm.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.ash-vs-three-vrm.diff.png" width="160"> |

## Generated glTF PBR Fallback

This fixture deliberately has no `VRMC_materials_mtoon`; it checks the glTF/PBR fallback path: base color, texture, roughness, metallic, emissive strength, occlusion, unlit, texture-factor, and normal-map slots.

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/three-vrm/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/wgpu/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/bevy/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/ash/gltf-pbr_vrm.frame000.png" width="192"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.wgpu-vs-three-vrm.diff.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.bevy-vs-three-vrm.diff.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.ash-vs-three-vrm.diff.png" width="192"> |

## Current Blocker Images

The expanded2 diagnostic directory has PNG readbacks for wgpu, Bevy, and Ash. Ash PNGs are generated from byte-equivalent `.rgba.json` artifacts with `just render-parity-current-ash-pngs`. These images should not be treated as a desired final behavior, because the latest notes reject blind owner-frontier expansion as a default fix.

| wgpu expanded2 readback | Bevy expanded2 readback | Ash expanded2 readback |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/wgpu/Seed-san.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/bevy/Seed-san.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/ash/Seed-san.frame000.png" width="192"> |

## Recommended Reading Order

1. Open the generated `visual-review.html` for the target artifact set.
2. Check `summary.md` when it exists.
3. Use this page to compare `three-vrm`, Rust renderer outputs, and diff PNGs in one Markdown view.
4. For remaining blocker work, prefer raw `.imqraw` comparisons and hotspot JSON over PNG-only inspection.
