# Current Rendering Result Comparison

Updated: 2026-06-22

This page compares the render images that exist in this workspace right now.
The image artifacts live under `.external-fixtures/` and are intentionally not
committed. The reference column is `three-vrm`; the other columns are Rust
renderers.

For numeric decisions, prefer the raw `.imqraw` reports linked from
[render-parity-current-images.md](render-parity-current-images.md). This file is
for quick visual review.

## Main Current Image Set

Artifact directory:
[`../.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun)

Metric: `rgb-visible`, opaque black background, exact alpha parity.

| Renderer | PSNR | Gradient-domain PSNR | Changed RGB pixels | Max channel delta | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: | ---: |
| `wgpu` | 36.8913 | 32.6698 | 1107 | 251 | 0 |
| Bevy | 36.8708 | 32.6368 | 9252 | 251 | 0 |
| Ash | 36.8913 | 32.6698 | 1107 | 251 | 0 |

| three-vrm reference | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/three-vrm/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/wgpu/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/bevy/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/ash/Seed-san.frame000.png" width="190"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.ash-vs-three-vrm.diff.png" width="190"> |

Reading: this set passes the current visual gate, and alpha/readback format are
not the blocker. The remaining visible gap is localized around material color,
texture sampling, and edge/owner behavior.

## Real Sample Sweep

Artifact directory:
[`../.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check)

Metric: `rgb-visible`.

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | Current status |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | Passes gate; still the main real-model color/ownership target. |
| `VRM1_Constraint_Twist_Sample.vrm` | 36.2518 | 36.2349 | 36.2509 | Passes; residuals are local. |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | Passes; UV animation path is covered. |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | 55.6968 | 55.2106 | 55.6968 | Strong parity. |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | 55.7181 | 55.2306 | 55.7181 | Strong parity. |
| `AliciaSolid_vrm-0.51.vrm` | 35.6238 | 35.6088 | 35.6238 | Passes; VRM0 compatibility path is covered. |

### Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/Seed-san.frame000.png" width="175"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.ash-vs-three-vrm.diff.png" width="175"> |

### Constraint And UV Samples

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Constraint | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> |
| MToon UV animation | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> |

### Expressions And VRM0

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Expression overridden | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> |
| Expression overrides | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> |
| AliciaSolid VRM0 | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/AliciaSolid_vrm-0_51.frame000.png" width="130"> |

## Transparent And PBR Guards

| Fixture | Metric | wgpu | Bevy | Ash | Current status |
| --- | --- | ---: | ---: | ---: | --- |
| `transparent-blend_vrm` | `rgb-visible` | 54.3997 | 56.8605 | 54.3997 | Exact alpha parity, max channel delta 1. |
| `gltf-pbr_vrm` | `rgb-interior1px` | 47.8937 | 47.4289 | 47.8937 | Non-MToon glTF/PBR fallback is guarded, including a Seed-san-like backpack swatch. |

### Generated Transparent Blend

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/three-vrm/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/wgpu/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/bevy/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/ash/transparent-blend_vrm.frame000.png" width="175"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.wgpu-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.bevy-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.ash-vs-three-vrm.diff.png" width="175"> |

### Generated glTF/PBR Fallback

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/three-vrm/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/wgpu/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/bevy/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/ash/gltf-pbr_vrm.frame000.png" width="200"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.wgpu-vs-three-vrm.diff.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.bevy-vs-three-vrm.diff.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.ash-vs-three-vrm.diff.png" width="200"> |

## Current Blocker Diagnostic

The remaining Seed-san base-color diagnostic should be read as evidence, not as
a default behavior target. The second-frontier run is a negative control:
blindly expanding ownership improves some pixels and regresses others.

After the latest quad-resolve update, the expanded readback keeps exact alpha
parity across wgpu, Bevy, and Ash. Ash PNGs can be regenerated from the existing
`.rgba.json` artifacts with `just render-parity-current-ash-pngs`.

| Set | wgpu gradient PSNR | Bevy gradient PSNR | Ash gradient PSNR | Use |
| --- | ---: | ---: | ---: | --- |
| Current readback | 30.6336 | 28.2142 | 35.8902 | Current source of truth. |
| Expanded post-resolve diagnostic, current local artifact | 32.7302 | 32.4826 | 35.8901 | Target-pixel coverage diagnostic; Bevy now uses the UV0/UV1 gradient path. |
| Second-frontier negative control | 30.9642 | 28.6200 | 35.8901 | Regression guard, not a fix. |

| three-vrm reference | wgpu current readback | Bevy current readback | Ash current readback |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/wgpu/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/bevy/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/ash/Seed-san.frame000.png" width="185"> |

| wgpu expanded diagnostic | Bevy expanded diagnostic | Ash expanded diagnostic |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.png" width="180"> |

| wgpu second-frontier negative control | Bevy second-frontier negative control | Ash second-frontier negative control |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/ash/Seed-san.frame000.png" width="180"> |

### Expanded Material/Color Diagnostic

The `target/texture-draw-audit/` expected-vs-actual (`E-A`) reports show that
the selected bucket mean E-A distance is wgpu `45.89`, Bevy `45.35`, and Ash
`36.97`. The direction differs by material and draw key, so this is not a good
candidate for another global exposure or color-space toggle.

| Renderer | Selected mean E-A | Representative expected-brighter bucket | Representative expected-darker bucket | Reading |
| --- | ---: | --- | --- | --- |
| wgpu | 45.89 | `backpack_nm node145/mesh4/prim9/base` `+18.47,+21.00,+22.33` | `body_nm node145/mesh4/prim1/base` `-33.00,-26.50,-24.75` | Split glTF/PBR backpack work from MToon body/plastic work. |
| Ash | 36.97 | `backpack_nm node145/mesh4/prim9/base` `+16.57,+18.83,+19.83` | `body_nm node145/mesh4/prim1/base` `-9.50,-7.88,-7.75` | Same directional split as wgpu; not backend-only. |
| Bevy | 45.35 | Expected-brighter residuals around `backpack_nm` | Local `body_nm` / plastic residuals | Read it on the same material/draw-key axis as wgpu and Ash. |

The real shading-model join Markdown and the expanded summary JSON now agree:
`color_fit` is present in JSON instead of serializing as `null`. The parser
accepts both `additive_fit_mean_rgb_distance` / `gain_fit_mean_rgb_distance`
and the shorter `additive_fit_mean_distance` / `gain_fit_mean_distance` field
names.

Renderer artifacts also expose `materialExtra.shaderBranch` as `gltf_pbr`,
`mtoon`, or `unlit`. Prefer that explicit branch field over the older compact
`pbr:*` text in stale focused reports when reading material/draw diagnostics.

Audit Markdown:

- [`target/texture-draw-audit/Seed-san.wgpu.expected-actual.md`](../target/texture-draw-audit/Seed-san.wgpu.expected-actual.md)
- [`target/texture-draw-audit/Seed-san.bevy.expected-actual.md`](../target/texture-draw-audit/Seed-san.bevy.expected-actual.md)
- [`target/texture-draw-audit/Seed-san.ash.expected-actual.md`](../target/texture-draw-audit/Seed-san.ash.expected-actual.md)
- [`Seed-san.render-resolve-expanded.summary.json`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.json)

## Review Order

1. Start with `Main Current Image Set` for the latest whole-image state.
2. Check `Real Sample Sweep` to make sure broad parity is still intact.
3. Use `Transparent And PBR Guards` to confirm alpha and non-MToon paths.
4. Use `Current Blocker Diagnostic` only for targeted material/color ownership
   work; do not treat expanded diagnostics as desired renderer behavior.
5. Use the E-A audit rows to split glTF/PBR backpack color accumulation from
   MToon body/plastic local material differences.
