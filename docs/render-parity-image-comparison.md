# Rendering Image Comparison

Updated: 2026-06-21

This page is a local visual index for the current render-parity artifacts. The PNG and `.imqraw` files are intentionally kept under `.external-fixtures/` and are not source-controlled; regenerate them with the listed `just` recipes before using this page on another machine.

The reference image is `three-vrm`. The compared images are Rust renderers: `wgpu`, `bevy`, and `ash`. Diff images are generated against `three-vrm`.

## Current Artifact Sets

| Set | Recipe | Artifact directory | Review page | Metric |
| --- | --- | --- | --- | --- |
| Real samples, opaque black | `just render-parity-samples-ash-gated` | [`.external-fixtures/render-parity-samples-ash-gated`](../.external-fixtures/render-parity-samples-ash-gated) | [`visual-review.html`](../.external-fixtures/render-parity-samples-ash-gated/visual-review.html) | `rgb-visible` |
| Real samples, transparent | `just render-parity-real-transparent-ash-gated` | [`.external-fixtures/render-parity-real-transparent-ash-gated`](../.external-fixtures/render-parity-real-transparent-ash-gated) | [`visual-review.html`](../.external-fixtures/render-parity-real-transparent-ash-gated/visual-review.html) | `rgb-all` |
| Generated glTF PBR fallback | `just render-parity-gltf-pbr-generated` | [`.external-fixtures/render-parity-gltf-pbr-generated`](../.external-fixtures/render-parity-gltf-pbr-generated) | [`visual-review.html`](../.external-fixtures/render-parity-gltf-pbr-generated/visual-review.html) | `rgb-interior1px` |
| Current Seed-san owner/sample blocker | `just render-parity-current-blocker` | [`.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback) | N/A | diagnostic |

## Numeric Snapshot

### Real Samples, Opaque Black

Gate: exact alpha parity, `rgb-visible >= 34 dB`, Ash included in visual gate.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Current reading |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | passes, but still the main real-model texture/material ownership target |
| `VRM1_Constraint_Twist_Sample.vrm` | 36.2518 | 36.2349 | 36.2509 | passes; local edge/material ownership remains visible in diagnostics |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | passes; UV animation parity is guarded |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | 55.6968 | 55.2106 | 55.6968 | strong parity |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | 55.7181 | 55.2306 | 55.7181 | strong parity |
| `AliciaSolid_vrm-0.51.vrm` | 35.6238 | 35.6088 | 35.6238 | passes; VRM0 compatibility remains covered |

### Real Samples, Transparent

Gate: `rgb-all >= 32 dB`, alpha mismatch tolerance `64` pixels.

| Fixture | wgpu alpha mismatches | Bevy alpha mismatches | Ash alpha mismatches | Current reading |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 25 | 32 | 25 | transparent path is unified enough for the current gate |
| `VRM1_Constraint_Twist_Sample.vrm` | 11 | 11 | 11 | transparent path matches across Rust renderers |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 0 | 0 | 0 | exact alpha parity |
| expression samples | 3 | 3 | 3 | stable tiny transparent edge set |
| `AliciaSolid_vrm-0.51.vrm` | 32 | 32 | 32 | stable within current tolerance |

### Generated glTF PBR Fallback

Gate: `rgb-interior1px >= 48 dB`, max selected channel delta `<= 3`, per-swatch `>= 40 dB`.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Current reading |
| --- | ---: | ---: | ---: | --- |
| `gltf-pbr.vrm.gltf` | 49.2238 | 48.5934 | 49.2238 | broad non-MToon glTF/PBR fallback is not the Seed-san blocker |

## Real Sample Sweep

### Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/Seed-san.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/Seed-san.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/Seed-san.ash-vs-three-vrm.diff.png" width="160"> |

### VRM1 Constraint Twist Sample

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/VRM1_Constraint_Twist_Sample.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRM1_Constraint_Twist_Sample.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRM1_Constraint_Twist_Sample.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRM1_Constraint_Twist_Sample.ash-vs-three-vrm.diff.png" width="160"> |

### MToon UV Animation Test

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRMC_materials_mtoon_UV_Animation_Test.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRMC_materials_mtoon_UV_Animation_Test.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/VRMC_materials_mtoon_UV_Animation_Test.ash-vs-three-vrm.diff.png" width="160"> |

### Expression Samples

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Overridden | <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="128"> |
| Overrides | <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="128"> |

### AliciaSolid VRM0

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/bevy/AliciaSolid_vrm-0_51.frame000.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/ash/AliciaSolid_vrm-0_51.frame000.png" width="160"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/AliciaSolid_vrm-0_51.wgpu-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/AliciaSolid_vrm-0_51.bevy-vs-three-vrm.diff.png" width="160"> | <img src="../.external-fixtures/render-parity-samples-ash-gated/diff/AliciaSolid_vrm-0_51.ash-vs-three-vrm.diff.png" width="160"> |

## Transparent Sweep Spot Check

Use this section when checking alpha behavior. The RGB values are intentionally the same parity target as the opaque sweep, but the transparent artifacts expose edge-alpha disagreements.

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Seed-san | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/three-vrm/Seed-san.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/wgpu/Seed-san.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/bevy/Seed-san.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/ash/Seed-san.frame000.png" width="128"> |
| AliciaSolid | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/bevy/AliciaSolid_vrm-0_51.frame000.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/ash/AliciaSolid_vrm-0_51.frame000.png" width="128"> |

| Fixture | wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- | --- |
| Seed-san | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/Seed-san.ash-vs-three-vrm.diff.png" width="128"> |
| AliciaSolid | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/AliciaSolid_vrm-0_51.wgpu-vs-three-vrm.diff.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/AliciaSolid_vrm-0_51.bevy-vs-three-vrm.diff.png" width="128"> | <img src="../.external-fixtures/render-parity-real-transparent-ash-gated/diff/AliciaSolid_vrm-0_51.ash-vs-three-vrm.diff.png" width="128"> |

## Generated glTF PBR Fallback

This fixture deliberately has no `VRMC_materials_mtoon`; it checks the glTF/PBR fallback path: base color, texture, roughness, metallic, emissive strength, occlusion, unlit, and texture-factor slots.

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/three-vrm/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/wgpu/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/bevy/gltf-pbr_vrm.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/ash/gltf-pbr_vrm.frame000.png" width="192"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.wgpu-vs-three-vrm.diff.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.bevy-vs-three-vrm.diff.png" width="192"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.ash-vs-three-vrm.diff.png" width="192"> |

## Current Blocker Image

The current real-model blocker is not a broad PBR fallback failure. It is still concentrated around Seed-san local owner/fill/material-color behavior. The most useful quick image check is the render-resolve readback artifact below; use the JSON reports in the same directory for detailed hotspot causality.

| wgpu | Bevy |
| --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/wgpu/Seed-san.frame000.png" width="192"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/bevy/Seed-san.frame000.png" width="192"> |

## Recommended Reading Order

1. Open the generated `visual-review.html` for the target artifact set.
2. Check the table in the corresponding `summary.md`.
3. Use this page to compare the reference, Rust renderer outputs, and diff PNGs in one Markdown view.
4. For remaining blocker work, prefer raw `.imqraw` comparisons and hotspot JSON over PNG-only inspection.
