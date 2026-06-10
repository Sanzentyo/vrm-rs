# imq Compare Requirements For VRM Render Parity

This document records the `compare-psnr.mjs` behavior that should move into
`imq` before `vrm-rs` can replace the local JavaScript comparator with direct
`imqraw` renderer output and `imq` metric reports.

The target is not just generic image PSNR. VRM render parity needs metrics that
separate model-body color drift, transparent-material alpha drift, and one-pixel
silhouette or fill-rule differences.

## Latest imq Check

Checked locally on 2026-06-10 after reinstalling the latest public `imq` main:

```powershell
cargo install --git https://github.com/Sanzentyo/imq.git imq --locked --force
imq --version
```

The installed revision was `0fdc5263` and still reports version `0.1.0`. The
standard raw `imqraw` path works for `psnr:color`, `mse:color`, `mae:color`,
`maxae:color`, `psnr:all`, and `mse:all`; the Seed-san wgpu-vs-three-vrm
normal-map artifact reports `psnr:color = 34.6181114203507 dB`.

The render-parity domains documented by the refreshed `imq-cli` skill were not
present in that installed CLI yet:

```text
Error: unsupported metric input: unknown sample domain `rgb-visible`
```

Until an `imq` revision exposes those domains and threshold gates in the actual
CLI/crate, `compare-psnr.mjs` remains the authoritative VRM-specific gate and
`imq` remains an independent raw-buffer cross-check for generic color/all
metrics.

## Current Comparator Role

`tools/render-parity/compare-psnr.mjs` currently compares two `.rgba.json`
artifacts:

```json
{
  "width": 256,
  "height": 256,
  "rgba": [255, 0, 0, 255]
}
```

The renderer outputs already contain raw top-left-origin RGBA8 readback data.
The comparator does not compare PNG files. The long-term improvement is to have
the JavaScript three-vrm reference and the Rust wgpu/Bevy captures emit
`imqraw` directly, removing the JSON number-array serialization cost.

## Required Metric Domains

### `rgba`

Compare every stored RGBA channel for every pixel.

Use this for strict raw-buffer checks and old full-image regressions.

### `rgb-all`

Compare RGB for every pixel and ignore alpha.

Use this when alpha is validated separately but full-canvas RGB drift still
matters.

### `rgb-opaque`

Compare RGB only where both reference and candidate have `alpha == 255`.

Use this for stable opaque-surface color checks that should ignore transparent
edges and partial-alpha material regions.

### `rgb-visible`

Compare RGB where either reference or candidate has `alpha > 0`.

Use this for visible-surface checks on transparent backgrounds and alpha-mask
audits.

### `rgb-nonblack`

Compare RGB where either reference or candidate has any non-zero RGB channel.

Use this for opaque-black review backgrounds so empty black canvas pixels do
not dilute model-body error.

## Required One-Pixel Interior Domains

All interior domains must drop the outer image border and require a stable
3-by-3 neighborhood around the selected pixel.

### `rgb-interior1px`

Compare RGB only where every pixel in the 3-by-3 neighborhood is opaque in both
images.

Use this for generated opaque swatches and material color audits where one-pixel
rasterization/fill differences should not dominate PSNR.

### `rgb-visible-interior1px`

Compare RGB only where every pixel in the 3-by-3 neighborhood is visible
(`alpha > 0`) in both images.

Use this for transparent-material interior checks, including partial-alpha
regions, while excluding silhouette edges.

### `rgb-nonblack-interior1px`

Compare RGB only where every pixel in the 3-by-3 neighborhood is non-black in
either image.

Use this for opaque-black model-body diagnostics that should ignore both empty
background and one-pixel silhouettes.

## Alpha Diagnostics

Reports must include alpha diagnostics independently from RGB metrics:

- Reference alpha bucket counts:
  - `transparent`: `alpha == 0`
  - `opaque`: `alpha == 255`
  - `partial`: `0 < alpha < 255`
- Candidate alpha bucket counts.
- Alpha mismatch count.
- Maximum alpha delta.
- Alpha mismatches beyond one least-significant bit.

These fields let render parity distinguish transparent-material policy drift
from RGB shading drift.

## Per-Metric Diagnostics

Each metric domain should report:

- Pixel count.
- Channel sample count.
- MSE.
- PSNR.
- MAE.
- Maximum channel delta.
- Maximum pixel delta.

`max pixel delta` should be the Euclidean distance over the selected channels
for one pixel. This catches localized severe errors that aggregate PSNR can
hide.

## Selected Metric And Thresholds

`imq` should support calculating several domains while selecting one domain for
pass/fail:

```powershell
imq image reference candidate `
  --metrics psnr:rgb-visible,mse:rgb-visible,mae:rgb-visible,maxae:rgb-visible `
  --selected-metric rgb-visible `
  --fail-under 34 `
  --max-selected-channel-delta 4 `
  --max-alpha-delta 1
```

Required gate inputs:

- `--selected-metric <DOMAIN>`
- `--fail-under <DB>`
- `--max-selected-channel-delta <0..255>`
- `--max-alpha-delta <0..255>`
- Future: `--max-alpha-mismatches <COUNT>`

The report should include the selected metric, thresholds, and final boolean
pass status.

## imqraw Renderer Integration

The final render parity path should avoid `.rgba.json` for numeric comparison:

- three-vrm JavaScript/WebGL reference:
  - use `encodeThreeRenderer`, or `gl.readPixels` plus `encodeRgba8` /
    `encodeBundle`.
  - Current status: the browser capture accepts `--imqraw-out` and encodes the
    top-left RGBA readback through `encodeRgba8` with `three-vrm`/`reference`
    tags.
- Rust wgpu/Bevy captures:
  - use the `imq` Rust crate to encode `FrameOwned::packed_tight(...,
    PixelFormat::Rgba8)` records.
  - Current status: wgpu and Bevy examples accept `--imqraw-out` and local
    render parity writes those `.imqraw` files beside `.rgba.json`.
- Local runner:
  - compare tagged reference/candidate records from `imqraw` without PNG or
    decimal JSON conversion.
  - Current status: `tools/render-parity/compare-imqraw.rs` already compares
    direct single-image renderer `.imqraw` bundles and emits
    `.imqraw-rust.json` reports with the same local VRM domains as
    `compare-psnr.mjs`. The main pass/fail gate still uses `.psnr.json` until
    public `imq` exposes those domains and thresholds directly.

PNG and HTML artifacts should remain review artifacts only. Numeric parity
should operate on raw RGBA8 data.

## JSON Report Shape

The structured report should include:

- Reference label/tag/path.
- Candidate label/tag/path.
- Width, height, pixel format, and color/transfer metadata when available.
- Alpha diagnostics.
- All computed metric domains.
- Selected metric.
- Thresholds.
- Pass/fail status.

The field names do not need to be byte-for-byte compatible with
`compare-psnr.mjs`, but `tools/ci/local-ci.rs` should be able to consume the
report without lossy text parsing.

## Priority

1. Add `rgba`, `rgb-all`, `rgb-opaque`, and `rgb-visible` domains.
2. Add alpha bucket and alpha mismatch diagnostics.
3. Add selected-metric threshold gates.
4. Add `rgb-interior1px` and `rgb-visible-interior1px`.
5. Add `rgb-nonblack` and `rgb-nonblack-interior1px`.
6. Switch local render-parity numeric reports to direct renderer `imqraw`
   inputs after the installed `imq` CLI supports the required VRM metric
   domains and gates.
7. Retire `compare-psnr.mjs` after render-parity recipes no longer depend on
   metrics that only exist in the local comparator.
