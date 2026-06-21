# Current Render Image Comparison

Snapshot date: 2026-06-22

This note records the current image artifacts used while chasing three-vrm
render parity for `Seed-san.vrm`. The image files live under
`.external-fixtures/` and are intentionally not committed.

## Primary Readback Set

Artifact root:
`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback`

Summary:
`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.md`

Raw summary JSON:
`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.json`

This is the current cross-backend readback set with `wgpu`, `bevy`, and `ash`
outputs. Its three-vrm side in this directory is retained as hotspot projection
RGBA JSON rather than a full-frame PNG, so the full-frame visual baseline below
uses the latest three-vrm-backed diagnostic set.

| Renderer | Current PNG | Raw image | RGBA JSON |
| --- | --- | --- | --- |
| wgpu | ![wgpu Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.png) | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.imqraw` | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.rgba.json` |
| bevy | ![bevy Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.png) | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.imqraw` | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.rgba.json` |
| ash | ![ash Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.png) | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.imqraw` | `../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.rgba.json` |

Current metric: `rgbSharedNonblackGradientInterior1px`.

| Renderer | PSNR | Changed RGB | Max delta | Alpha mismatches | Selected hotspots | Missing selection | Main residual |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| wgpu | 32.7302 | 3499 | 237 | 0 | 39 | 25 | `backpack_nm` glTF/PBR is still brighter in three-vrm by additive RGB about `18.50,20.83,22.25` on `node145/mesh4/prim9/base`. |
| bevy | 32.4826 | 10014 | 237 | 0 | 40 | 24 | Same `backpack_nm` residual, additive RGB about `18.23,20.46,21.69`. |
| ash | 35.8901 | 3483 | 237 | 0 | 64 | 0 | Same residual class, smaller but still visible: additive RGB about `17.06,19.12,20.19`. |

Important interpretation: the additive/gain fit numbers are diagnostics, not
shader knobs. They show the current direction of the residual. Default behavior
should continue to be driven by source-matching material/light/texture semantics,
not by post-hoc gain tuning.

## Full-Frame three-vrm Baseline

Artifact root:
`../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic`

This is the latest local diagnostic set that still carries a full-frame
three-vrm PNG alongside `wgpu` and `bevy` PNGs plus visual diff PNGs.

| Image | Path | Preview |
| --- | --- | --- |
| three-vrm reference | `../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png` | ![three-vrm Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png) |
| wgpu | `../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/wgpu/Seed-san.frame000.png` | ![wgpu diagnostic Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/wgpu/Seed-san.frame000.png) |
| bevy | `../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/bevy/Seed-san.frame000.png` | ![bevy diagnostic Seed-san](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/bevy/Seed-san.frame000.png) |
| wgpu diff | `../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/diff/Seed-san.wgpu-vs-three-vrm.diff.png` | ![wgpu diff](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/diff/Seed-san.wgpu-vs-three-vrm.diff.png) |
| bevy diff | `../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/diff/Seed-san.bevy-vs-three-vrm.diff.png` | ![bevy diff](../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/diff/Seed-san.bevy-vs-three-vrm.diff.png) |

Diagnostic summary:
`../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/summary.md`

| Renderer | Selected metric | Selected PSNR | Max delta | Alpha mismatches | Visual gate |
| --- | --- | ---: | ---: | ---: | --- |
| wgpu | `rgb-shared-nonblack-flat32-interior1px` | 49.7607 | 29 | 0 | pass |
| bevy | `rgb-shared-nonblack-flat32-interior1px` | 47.5483 | 29 | 0 | pass |

The diagnostic full-frame set is useful for visual review, but it is not the
same measurement as the primary expanded readback set. Do not mix its PSNR with
the expanded readback PSNR when evaluating regressions.

## Color-Fit Parser Guard

The real join Markdown contains additive/gain fit rows for the current residuals.
The summary JSON must not collapse those rows to `color_fit: null`.

Current guardrail:

- `tools/render-parity/summarize-render-resolve-expanded.rs` accepts
  `color_fit`, `color_fit_summary`, `colorFit`, and `colorFitSummary`.
- If `color_fit` is null but an alias contains the fit, the alias is used.
- Summary JSON omits missing `color_fit` values instead of serializing them as
  `null`.
- The self-test asserts the parsed additive/gain fit values and checks that the
  serialized summary JSON does not contain `"color_fit":null`.

## Current Blocker

The backend images are structurally close enough for owner/alpha checks, but the
remaining blocker is material/light color accumulation. The strongest current
signal is the `backpack_nm` glTF/PBR residual on `node145/mesh4/prim9/base`:
three-vrm remains consistently brighter than the Rust renderers after ownership,
base-color projection, normal-map sampling, and render-readback diagnostics.

Next useful diagnostic: add a source-derived three.js `MeshStandardMaterial`
term dump for the same pixels and compare it against Rust CPU `pbr_terms`
without introducing any gain/exposure tuning.
