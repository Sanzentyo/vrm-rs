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
reads RGBA bytes from the WebGL drawing buffer. The next Bevy and wgpu capture
paths should match this camera/light setup before comparing PSNR.

## Review Criteria

Initial thresholds are intentionally conservative until real renderer captures
are stable:

- Static pose, unlit or neutral-light setup: PSNR >= 45 dB.
- MToon lit material render with antialiasing disabled: PSNR >= 40 dB.
- Spring/VRMA animated frame with deterministic camera and fixed timestep:
  PSNR >= 38 dB.

Any failure should store the expected, actual, difference/heatmap image if
available, and the PSNR report under `.external-fixtures/render-parity/reports/`.
Human visual review should compare the rendered PNGs alongside the numeric
report before declaring parity.

## Next Renderer Work

- Add a Bevy capture path that renders the same camera/light/material setup and
  writes RGBA JSON after readback.
- Add a wgpu capture path using `HeadlessSceneState` plus the MToon descriptor
  example layout, then write RGBA JSON from a staging buffer.
