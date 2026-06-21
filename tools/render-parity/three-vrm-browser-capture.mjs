#!/usr/bin/env node

import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import process from 'node:process';
import zlib from 'node:zlib';

const args = new Map();
const expressions = [];
for (let index = 2; index < process.argv.length; index += 1) {
  const arg = process.argv[index];
  if (!arg.startsWith('--')) continue;
  const key = arg.slice(2);
  const next = process.argv[index + 1];
  if (next == null || next.startsWith('--')) {
    args.set(key, 'true');
  } else {
    args.set(key, next);
    if (key === 'expression') expressions.push(next);
    index += 1;
  }
}

const fixture = args.get('fixture');
const threeVrmRoot = path.resolve(args.get('three-vrm-root') ?? '../three-vrm');
const out = args.get('out');
const pngOut = args.get('png-out');
const imqrawOut = args.get('imqraw-out');
const hotspotDeltasPath = args.get('hotspot-deltas');
const hotspotTop = Number.parseInt(args.get('hotspot-top') ?? '32', 10);
const hotspotSampleCenterX = Number(args.get('hotspot-sample-center-x') ?? '0.5');
const hotspotSampleCenterY = Number(args.get('hotspot-sample-center-y') ?? '0.5');
const hotspotSubpixelSteps = Number.parseInt(args.get('hotspot-subpixel-steps') ?? '3', 10);
const width = Number.parseInt(args.get('width') ?? '512', 10);
const height = Number.parseInt(args.get('height') ?? '512', 10);
const cameraY = Number(args.get('camera-y') ?? '1.0');
const cameraZ = Number(args.get('camera-z') ?? '5.0');
const targetY = Number(args.get('target-y') ?? '1.0');
const mtoonTime = Number(args.get('mtoon-time') ?? '0.0');
const directionalIntensity = Number(args.get('directional-intensity') ?? Math.PI.toString());
const directionalX = Number(args.get('directional-x') ?? '1.0');
const directionalY = Number(args.get('directional-y') ?? '1.0');
const directionalZ = Number(args.get('directional-z') ?? '1.0');
const directionalR = Number(args.get('directional-r') ?? '1.0');
const directionalG = Number(args.get('directional-g') ?? '1.0');
const directionalB = Number(args.get('directional-b') ?? '1.0');
const ambientIntensity = Number(args.get('ambient-intensity') ?? '0.1');
const background = args.get('background') ?? 'opaque-black';
const disableOutlines = args.has('disable-outlines');
const disableNormalMaps = args.has('disable-normal-maps');
const disableTextureMips = args.has('disable-texture-mips');
const forceNearestTextures = args.has('force-nearest-textures');
const diagnosticRender = args.get('diagnostic-render') ?? 'shaded';
const expressionWeights = parseExpressionWeights(expressions);
const hotspotDeltas = hotspotDeltasPath ? readHotspotDeltas(hotspotDeltasPath, hotspotTop) : null;

if (!fixture || !out) {
  console.error('usage: node tools/render-parity/three-vrm-browser-capture.mjs --fixture avatar.vrm --three-vrm-root ../three-vrm --out frame.rgba.json [--png-out frame.png] [--imqraw-out frame.imqraw] [--hotspot-deltas deltas.json] [--hotspot-top 32] [--hotspot-subpixel-steps 3] [--width 512] [--height 512] [--background opaque-black|transparent] [--ambient-intensity 0.1] [--directional-intensity PI] [--directional-r 1.0] [--expression happy=1.0] [--disable-outlines] [--disable-normal-maps] [--disable-texture-mips] [--force-nearest-textures] [--diagnostic-render shaded|flat|base-factor|base-color|base-color-flip-v|base-color-raw-srgb|uv|base-uv|owner-id]');
  process.exit(2);
}
if (![width, height].every((value) => Number.isInteger(value) && value > 0)) {
  console.error(`invalid dimensions: ${width}x${height}`);
  process.exit(2);
}
if (!Number.isInteger(hotspotTop) || hotspotTop <= 0) {
  console.error(`invalid hotspot-top: ${hotspotTop}`);
  process.exit(2);
}
if (!Number.isInteger(hotspotSubpixelSteps) || hotspotSubpixelSteps <= 0) {
  console.error(`invalid hotspot-subpixel-steps: ${hotspotSubpixelSteps}`);
  process.exit(2);
}
if (![hotspotSampleCenterX, hotspotSampleCenterY].every(Number.isFinite)) {
  console.error('hotspot sample center values must be finite numbers');
  process.exit(2);
}
if (
  ![
    cameraY,
    cameraZ,
    targetY,
    mtoonTime,
    directionalIntensity,
    directionalX,
    directionalY,
    directionalZ,
    directionalR,
    directionalG,
    directionalB,
    ambientIntensity,
  ].every(Number.isFinite)
) {
  console.error('camera, mtoon-time, and light parameters must be finite numbers');
  process.exit(2);
}
if (directionalX === 0 && directionalY === 0 && directionalZ === 0) {
  console.error('directional light vector must not be zero');
  process.exit(2);
}
if (!['opaque-black', 'transparent'].includes(background)) {
  console.error(`invalid background: ${background}; expected opaque-black or transparent`);
  process.exit(2);
}
if (!['shaded', 'flat', 'base-factor', 'base-color', 'base-color-flip-v', 'base-color-raw-srgb', 'uv', 'base-uv', 'owner-id'].includes(diagnosticRender)) {
  console.error(`invalid diagnostic-render: ${diagnosticRender}; expected shaded, flat, base-factor, base-color, base-color-flip-v, base-color-raw-srgb, uv, base-uv, or owner-id`);
  process.exit(2);
}

let chromium;
try {
  ({ chromium } = await import('playwright'));
} catch (error) {
  console.error('Playwright is required for browser capture. Install it outside the repo source tree, for example: npm install --no-save playwright');
  console.error(error?.message ?? error);
  process.exit(2);
}

const fixturePath = path.resolve(fixture);
const threePackage = path.join(threeVrmRoot, 'packages/three-vrm');
const threeModuleRoot = path.join(threePackage, 'node_modules/three');
const routes = new Map([
  ['/fixture.vrm', fixturePath],
  ['/three-vrm/lib/three-vrm.module.js', path.join(threePackage, 'lib/three-vrm.module.js')],
]);

for (const file of routes.values()) {
  if (!fs.existsSync(file)) {
    console.error(`required file does not exist: ${file}`);
    process.exit(2);
  }
}

const server = http.createServer((request, response) => {
  const url = new URL(request.url ?? '/', 'http://127.0.0.1');
  if (url.pathname === '/') {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    response.end(capturePage({
      width,
      height,
      cameraY,
      cameraZ,
      targetY,
      mtoonTime,
      expressions: expressionWeights,
      background,
      imqraw: Boolean(imqrawOut),
      directionalIntensity,
      directionalX,
      directionalY,
      directionalZ,
      directionalR,
      directionalG,
      directionalB,
      ambientIntensity,
      disableTextureMips,
      diagnosticRender,
      hotspotDeltas,
      hotspotSampleCenter: [hotspotSampleCenterX, hotspotSampleCenterY],
      hotspotSubpixelSteps,
    }));
    return;
  }

  const file = routes.get(url.pathname);
  if (file) {
    serveFile(response, file);
    return;
  }

  if (url.pathname.startsWith('/three/')) {
    const file = path.resolve(threeModuleRoot, `.${url.pathname.slice('/three'.length)}`);
    if (file.startsWith(path.resolve(threeModuleRoot)) && fs.existsSync(file)) {
      serveFile(response, file);
      return;
    }
  }

  {
    console.error(`browser requested unknown path: ${url.pathname}`);
    response.writeHead(404);
    response.end('not found');
    return;
  }
});

await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const { port } = server.address();

let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') {
      console.error(`browser ${message.type()}: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => {
    console.error(`browser pageerror: ${error.message}`);
  });
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => typeof globalThis.captureVrmFrame === 'function', null, {
    timeout: 15000,
  });
  const capture = await page.evaluate(() => globalThis.captureVrmFrame());
  const json = `${JSON.stringify({
    generator: 'vrm-rs tools/render-parity/three-vrm-browser-capture.mjs',
    fixture: path.resolve(fixturePath),
    threeVrmRoot,
    width,
    height,
    camera: { y: cameraY, z: cameraZ, targetY },
    disableOutlines,
    disableNormalMaps,
    disableTextureMips,
    reference: capture.reference,
    mtoonTime,
    expressions: expressionWeights,
    format: 'rgba8',
    rgba: capture.rgba,
  }, null, 2)}\n`;
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, json);
  if (pngOut) {
    fs.mkdirSync(path.dirname(pngOut), { recursive: true });
    fs.writeFileSync(pngOut, encodePngRgba(width, height, capture.rgba));
  }
  if (imqrawOut) {
    if (!Array.isArray(capture.imqraw)) {
      throw new Error('browser capture did not return imqraw bytes');
    }
    fs.mkdirSync(path.dirname(imqrawOut), { recursive: true });
    fs.writeFileSync(imqrawOut, Buffer.from(capture.imqraw));
  }
} finally {
  if (browser) await browser.close();
  await new Promise((resolve) => server.close(resolve));
}

function capturePage(options) {
  const transparent = options.background === 'transparent';
  const clearAlpha = transparent ? 0 : 1;
  const cssBackground = transparent ? 'transparent' : '#000';
  return `<!doctype html>
<meta charset="utf-8">
<style>
  html, body { margin: 0; background: ${cssBackground}; }
</style>
<canvas id="canvas" width="${options.width}" height="${options.height}" style="width:${options.width}px;height:${options.height}px;display:block"></canvas>
<script type="importmap">
  {
    "imports": {
      "three": "/three/build/three.module.js"
    }
  }
</script>
<script type="module">
  import * as THREE from '/three/build/three.module.js';
  import { GLTFLoader } from '/three/examples/jsm/loaders/GLTFLoader.js';
  import { VRMLoaderPlugin } from '/three-vrm/lib/three-vrm.module.js';
  ${options.imqraw ? "import { init as initImqraw, encodeRgba8 } from 'https://sanzentyo.github.io/imq/imqraw/v0.1.0/imqraw.js';" : ''}

  const textureReport = (texture) => {
    if (!texture?.isTexture) return null;
    if (texture.matrixAutoUpdate && typeof texture.updateMatrix === 'function') {
      texture.updateMatrix();
    }
    return {
      uuid: texture.uuid,
      name: texture.name,
      channel: texture.channel ?? 0,
      flipY: texture.flipY,
      colorSpace: texture.colorSpace,
      wrapS: texture.wrapS,
      wrapT: texture.wrapT,
      minFilter: texture.minFilter,
      magFilter: texture.magFilter,
      generateMipmaps: texture.generateMipmaps,
      matrixAutoUpdate: texture.matrixAutoUpdate,
      offset: texture.offset?.toArray?.() ?? null,
      repeat: texture.repeat?.toArray?.() ?? null,
      rotation: texture.rotation ?? 0,
      center: texture.center?.toArray?.() ?? null,
      matrix: texture.matrix?.elements ? Array.from(texture.matrix.elements) : null,
    };
  };

  const materialPass = (material) => (
    (material?.name ?? '').includes('(Outline)') || (material?.type ?? '').toLowerCase().includes('outline')
      ? 'outline'
      : 'base'
  );

  const materialReport = (material, mesh, slot) => ({
    meshName: mesh?.name ?? '',
    meshUuid: mesh?.uuid ?? null,
    materialSlot: slot,
    materialName: material?.name ?? '',
    pass: materialPass(material),
    materialUuid: material?.uuid ?? null,
    materialType: material?.type ?? null,
    side: material?.side ?? null,
    transparent: material?.transparent ?? false,
    opacity: material?.opacity ?? 1.0,
    alphaTest: material?.alphaTest ?? 0.0,
    depthWrite: material?.depthWrite ?? true,
    depthTest: material?.depthTest ?? true,
    blending: material?.blending ?? null,
    premultipliedAlpha: material?.premultipliedAlpha ?? false,
    color: material?.color?.isColor ? material.color.toArray() : null,
    map: textureReport(material?.map),
  });

  const attributeReport = (attribute) => {
    if (!attribute) return null;
    const array = attribute.array ?? [];
    const valueCount = Math.min(array.length, 24);
    return {
      itemSize: attribute.itemSize,
      count: attribute.count,
      normalized: attribute.normalized,
      arrayType: array.constructor?.name ?? null,
      firstValues: Array.from(array.slice?.(0, valueCount) ?? []),
    };
  };

  const geometryReport = (mesh) => {
    const geometry = mesh?.geometry;
    const materialNames = (Array.isArray(mesh?.material) ? mesh.material : [mesh?.material])
      .filter(Boolean)
      .map((material) => material.name ?? '');
    return {
      meshName: mesh?.name ?? '',
      meshUuid: mesh?.uuid ?? null,
      materialNames,
      index: attributeReport(geometry?.index),
      groups: (geometry?.groups ?? []).map((group) => ({
        start: group.start,
        count: group.count,
        materialIndex: group.materialIndex,
      })),
      attributes: {
        position: attributeReport(geometry?.attributes?.position),
        normal: attributeReport(geometry?.attributes?.normal),
        uv: attributeReport(geometry?.attributes?.uv),
        uv1: attributeReport(geometry?.attributes?.uv1),
        uv2: attributeReport(geometry?.attributes?.uv2),
        uv3: attributeReport(geometry?.attributes?.uv3),
      },
    };
  };

  const hotspotDeltas = ${JSON.stringify(options.hotspotDeltas)};
  const hotspotSampleCenter = ${JSON.stringify(options.hotspotSampleCenter)};
  const hotspotSubpixelSteps = ${JSON.stringify(options.hotspotSubpixelSteps)};
  const ownerIdRecords = [];
  const ownerIdByColor = new Map();
  const ownerIdByCandidate = new Map();
  const ownerIdByDiagnosticCandidate = new Map();
  let nextOwnerId = 1;

  const screenVertex = (mesh, index, uvAttribute, viewProjection, target, clip) => {
    mesh.getVertexPosition(index, target);
    target.applyMatrix4(mesh.matrixWorld);
    clip.set(target.x, target.y, target.z, 1.0).applyMatrix4(viewProjection);
    if (Math.abs(clip.w) <= Number.EPSILON) return null;
    const ndcX = clip.x / clip.w;
    const ndcY = clip.y / clip.w;
    const ndcZ = clip.z / clip.w;
    return {
      screen: [
        (ndcX * 0.5 + 0.5) * ${options.width},
        (0.5 - ndcY * 0.5) * ${options.height},
      ],
      depth: ndcZ,
      uv: uvAttribute ? [uvAttribute.getX(index), uvAttribute.getY(index)] : [0, 0],
      reciprocalW: 1.0 / clip.w,
    };
  };

  const barycentricWeights = (point, a, b, c) => {
    const denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if (Math.abs(denominator) <= 1.0e-5) return null;
    const w0 = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
    const w1 = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
    const w2 = 1.0 - w0 - w1;
    return [w0, w1, w2];
  };

  const barycentric = (point, a, b, c) => {
    const weights = barycentricWeights(point, a, b, c);
    if (!weights) return null;
    return weights[0] >= -1.0e-4 && weights[1] >= -1.0e-4 && weights[2] >= -1.0e-4
      ? weights
      : null;
  };

  const interpolateUv = (weights, a, b, c) => {
    const perspectiveWeights = [
      weights[0] * a.reciprocalW,
      weights[1] * b.reciprocalW,
      weights[2] * c.reciprocalW,
    ];
    const denominator = perspectiveWeights[0] + perspectiveWeights[1] + perspectiveWeights[2];
    if (Math.abs(denominator) <= Number.EPSILON) {
      return [
        weights[0] * a.uv[0] + weights[1] * b.uv[0] + weights[2] * c.uv[0],
        weights[0] * a.uv[1] + weights[1] * b.uv[1] + weights[2] * c.uv[1],
      ];
    }
    return [
      (perspectiveWeights[0] * a.uv[0] + perspectiveWeights[1] * b.uv[0] + perspectiveWeights[2] * c.uv[0]) / denominator,
      (perspectiveWeights[0] * a.uv[1] + perspectiveWeights[1] * b.uv[1] + perspectiveWeights[2] * c.uv[1]) / denominator,
    ];
  };

  const transformTextureUv = (uv, texture) => {
    if (!texture?.isTexture || !texture.matrix?.elements) return uv;
    if (texture.matrixAutoUpdate && typeof texture.updateMatrix === 'function') {
      texture.updateMatrix();
    }
    const e = texture.matrix.elements;
    return [
      e[0] * uv[0] + e[3] * uv[1] + e[6],
      e[1] * uv[0] + e[4] * uv[1] + e[7],
    ];
  };

  const linearToSrgb = (value) => {
    const clamped = Math.min(1, Math.max(0, value));
    return clamped <= 0.0031308 ? 12.92 * clamped : 1.055 * Math.pow(clamped, 1.0 / 2.4) - 0.055;
  };

  const quantize = (value) => Math.round(Math.min(1, Math.max(0, value)) * 255);

  const srgbToLinear = (value) => {
    const clamped = Math.min(1, Math.max(0, value));
    return clamped <= 0.04045 ? clamped / 12.92 : Math.pow((clamped + 0.055) / 1.055, 2.4);
  };

  const encodeOwnerId = (id) => [
    id & 0xff,
    (id >> 8) & 0xff,
    (id >> 16) & 0xff,
    255,
  ];

  const ownerColorKey = (rgba) => \`\${rgba[0]},\${rgba[1]},\${rgba[2]}\`;

  const ownerColorToLinearRgb = (rgba) => [
    srgbToLinear(rgba[0] / 255),
    srgbToLinear(rgba[1] / 255),
    srgbToLinear(rgba[2] / 255),
  ];

  const ownerCandidateKey = (meshUuid, materialIndex, triangle) => (
    \`\${meshUuid}:\${materialIndex}:\${triangle}\`
  );

  const texturePixelsCache = new Map();

  const texturePixels = (texture) => {
    if (!texture?.isTexture || !texture.image) return null;
    if (texturePixelsCache.has(texture.uuid)) return texturePixelsCache.get(texture.uuid);
    const image = texture.image;
    const width = image.width ?? image.naturalWidth ?? image.videoWidth ?? 0;
    const height = image.height ?? image.naturalHeight ?? image.videoHeight ?? 0;
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
      texturePixelsCache.set(texture.uuid, null);
      return null;
    }
    try {
      const canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      context.drawImage(image, 0, 0, width, height);
      const data = context.getImageData(0, 0, width, height).data;
      const pixels = { width, height, data };
      texturePixelsCache.set(texture.uuid, pixels);
      return pixels;
    } catch (error) {
      console.warn('failed to sample texture ' + (texture.name ?? texture.uuid) + ': ' + (error?.message ?? error));
      texturePixelsCache.set(texture.uuid, null);
      return null;
    }
  };

  const wrapCoord = (value, wrap) => {
    if (wrap === THREE.ClampToEdgeWrapping) return Math.min(1, Math.max(0, value));
    const repeated = value - Math.floor(value);
    if (wrap !== THREE.MirroredRepeatWrapping) return repeated;
    const mirror = value - Math.floor(value / 2) * 2;
    return mirror < 1 ? mirror : 2 - mirror;
  };

  const pixelAtRepeatLinear = (pixels, x, y, channel) => {
    const ix = ((x % pixels.width) + pixels.width) % pixels.width;
    const iy = ((y % pixels.height) + pixels.height) % pixels.height;
    return pixels.data[(iy * pixels.width + ix) * 4 + channel] ?? (channel === 3 ? 255 : 0);
  };

  const lerp = (left, right, amount) => left + (right - left) * amount;

  const sampleTextureRgba = (texture, uv) => {
    const pixels = texturePixels(texture);
    if (!pixels) return [255, 255, 255, 255];
    const u = wrapCoord(uv[0], texture.wrapS);
    const v = wrapCoord(uv[1], texture.wrapT);
    const x = u * pixels.width - 0.5;
    const y = v * pixels.height - 0.5;
    const x0 = Math.floor(x);
    const y0 = Math.floor(y);
    const tx = x - x0;
    const ty = y - y0;
    return [0, 1, 2, 3].map((channel) => quantize(lerp(
      lerp(
        pixelAtRepeatLinear(pixels, x0, y0, channel) / 255,
        pixelAtRepeatLinear(pixels, x0 + 1, y0, channel) / 255,
        tx,
      ),
      lerp(
        pixelAtRepeatLinear(pixels, x0, y0 + 1, channel) / 255,
        pixelAtRepeatLinear(pixels, x0 + 1, y0 + 1, channel) / 255,
        tx,
      ),
      ty,
    )));
  };

  const projectedBaseColor = (material, mapUv, alpha) => {
    const sampledMapRgba = sampleTextureRgba(material?.map, mapUv);
    const color = material?.color?.isColor ? material.color : { r: 1, g: 1, b: 1 };
    const projectedBaseColorSrgb = [
      quantize(linearToSrgb(color.r * srgbToLinear(sampledMapRgba[0] / 255))),
      quantize(linearToSrgb(color.g * srgbToLinear(sampledMapRgba[1] / 255))),
      quantize(linearToSrgb(color.b * srgbToLinear(sampledMapRgba[2] / 255))),
      alpha,
    ];
    return { sampledMapRgba, projectedBaseColorSrgb };
  };

  const rgbDistance = (left, right) => {
    const dr = left[0] - right[0];
    const dg = left[1] - right[1];
    const db = left[2] - right[2];
    return Math.sqrt(dr * dr + dg * dg + db * db);
  };

  const addUniquePoint = (points, point) => {
    if (!point || !point.every(Number.isFinite)) return;
    if (points.some((existing) => (
      Math.abs(existing[0] - point[0]) <= 1.0e-5
      && Math.abs(existing[1] - point[1]) <= 1.0e-5
    ))) return;
    points.push(point);
  };

  const pointInPixel = (point, x, y) => (
    point[0] >= x - 1.0e-4
    && point[0] <= x + 1.0 + 1.0e-4
    && point[1] >= y - 1.0e-4
    && point[1] <= y + 1.0 + 1.0e-4
  );

  const segmentIntersection = (a, b, c, d) => {
    const r = [b[0] - a[0], b[1] - a[1]];
    const s = [d[0] - c[0], d[1] - c[1]];
    const denominator = r[0] * s[1] - r[1] * s[0];
    if (Math.abs(denominator) <= 1.0e-7) return null;
    const delta = [c[0] - a[0], c[1] - a[1]];
    const t = (delta[0] * s[1] - delta[1] * s[0]) / denominator;
    const u = (delta[0] * r[1] - delta[1] * r[0]) / denominator;
    if (t < -1.0e-5 || t > 1.0 + 1.0e-5 || u < -1.0e-5 || u > 1.0 + 1.0e-5) {
      return null;
    }
    return [a[0] + t * r[0], a[1] + t * r[1]];
  };

  const pixelTriangleIntersectionPoint = (pixelX, pixelY, projected) => {
    const triangle = [projected.a.screen, projected.b.screen, projected.c.screen];
    const corners = [
      [pixelX, pixelY],
      [pixelX + 1, pixelY],
      [pixelX + 1, pixelY + 1],
      [pixelX, pixelY + 1],
    ];
    const points = [];
    for (const corner of corners) {
      if (barycentric(corner, triangle[0], triangle[1], triangle[2])) {
        addUniquePoint(points, corner);
      }
    }
    for (const vertex of triangle) {
      if (pointInPixel(vertex, pixelX, pixelY)) addUniquePoint(points, vertex);
    }
    for (let triEdge = 0; triEdge < 3; triEdge += 1) {
      const start = triangle[triEdge];
      const end = triangle[(triEdge + 1) % 3];
      for (let rectEdge = 0; rectEdge < 4; rectEdge += 1) {
        addUniquePoint(points, segmentIntersection(
          start,
          end,
          corners[rectEdge],
          corners[(rectEdge + 1) % 4],
        ));
      }
    }
    if (points.length === 0) return null;
    const point = points.reduce(
      (sum, current) => [sum[0] + current[0], sum[1] + current[1]],
      [0, 0],
    ).map((value) => value / points.length);
    const weights = barycentric(point, triangle[0], triangle[1], triangle[2])
      ?? barycentricWeights(point, triangle[0], triangle[1], triangle[2]);
    if (!weights) return null;
    return {
      point,
      barycentric: weights,
      pointCount: points.length,
      areaPixels: polygonAreaPixels(points),
    };
  };

  const polygonAreaPixels = (points) => {
    if (points.length < 3) return 0.0;
    const centroid = points.reduce(
      (sum, current) => [sum[0] + current[0], sum[1] + current[1]],
      [0, 0],
    ).map((value) => value / points.length);
    const ordered = points.slice().sort((left, right) => (
      Math.atan2(left[1] - centroid[1], left[0] - centroid[0])
      - Math.atan2(right[1] - centroid[1], right[0] - centroid[0])
    ));
    let twiceArea = 0.0;
    for (let index = 0; index < ordered.length; index += 1) {
      const left = ordered[index];
      const right = ordered[(index + 1) % ordered.length];
      twiceArea += left[0] * right[1] - right[0] * left[1];
    }
    return 0.5 * Math.abs(twiceArea);
  };

  const materialAt = (mesh, materialIndex) => (
    Array.isArray(mesh.material) ? mesh.material[materialIndex] : mesh.material
  );

  const triangleSignedArea = (a, b, c) => (
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
  );

  const gpuFrontFacingForFrontFace = (screenSignedArea, frontFace) => (
    frontFace === 'cw' ? screenSignedArea > 0.0 : screenSignedArea < 0.0
  );

  const threeCullMode = (material) => (
    (material?.side ?? THREE.FrontSide) === THREE.DoubleSide ? 'none' : 'back'
  );

  const threeFrontFace = (mesh, material) => {
    let flipSided = (material?.side ?? THREE.FrontSide) === THREE.BackSide;
    if (mesh?.isMesh && mesh.matrixWorld.determinant() < 0.0) {
      flipSided = !flipSided;
    }
    return flipSided ? 'cw' : 'ccw';
  };

  const visibleByThreeCullPolicy = (mesh, material, screenSignedArea) => {
    const cullMode = threeCullMode(material);
    if (cullMode === 'none') return true;
    const gpuFrontFacing = gpuFrontFacingForFrontFace(
      screenSignedArea,
      threeFrontFace(mesh, material),
    );
    return cullMode === 'back' ? gpuFrontFacing : !gpuFrontFacing;
  };

  const ownerTriangleProjection = (mesh, indices, viewProjection, target, clip) => {
    const projected = indices
      .map((index) => screenVertex(mesh, index, null, viewProjection, target, clip));
    if (projected.some((vertex) => vertex == null)) return null;
    const screen = projected.map((vertex) => vertex.screen);
    const screenSignedArea = triangleSignedArea(screen[0], screen[1], screen[2]);
    return {
      screen,
      screenBounds: {
        minX: Math.min(...screen.map((point) => point[0])),
        minY: Math.min(...screen.map((point) => point[1])),
        maxX: Math.max(...screen.map((point) => point[0])),
        maxY: Math.max(...screen.map((point) => point[1])),
      },
      depth: (projected[0].depth + projected[1].depth + projected[2].depth) / 3.0,
      webglDepth: (projected[0].depth + projected[1].depth + projected[2].depth) / 3.0,
      depthRange: 'webgl-ndc',
      screenSignedArea,
      frontFacing: screenSignedArea > 0.0,
    };
  };

  const attributeValue = (attribute, index, component) => {
    switch (component) {
      case 0: return attribute.getX(index);
      case 1: return attribute.getY(index);
      case 2: return attribute.getZ(index);
      case 3: return attribute.getW(index);
      default: return 0;
    }
  };

  const duplicateAttribute = (attribute, sourceVertexIndices) => {
    const array = new attribute.array.constructor(sourceVertexIndices.length * attribute.itemSize);
    for (let out = 0; out < sourceVertexIndices.length; out += 1) {
      const source = sourceVertexIndices[out];
      for (let component = 0; component < attribute.itemSize; component += 1) {
        array[out * attribute.itemSize + component] = attributeValue(attribute, source, component);
      }
    }
    return new THREE.BufferAttribute(array, attribute.itemSize, attribute.normalized);
  };

  const buildOwnerDiagnosticGeometry = (mesh, viewProjection) => {
    const sourceGeometry = mesh.geometry;
    const sourceIndex = sourceGeometry.index;
    const diagnosticGeometry = new THREE.BufferGeometry();
    const sourcePosition = sourceGeometry.attributes.position;
    const sourceVertexIndices = [];
    const colors = [];
    const target = new THREE.Vector3();
    const clip = new THREE.Vector4();
    const groups = sourceGeometry.groups.length > 0
      ? sourceGeometry.groups
      : [{ start: 0, count: sourceIndex ? sourceIndex.count : sourcePosition.count, materialIndex: 0 }];
    for (const group of groups) {
      const groupVertexStart = sourceVertexIndices.length;
      for (let offset = group.start; offset + 2 < group.start + group.count; offset += 3) {
        const id = nextOwnerId;
        nextOwnerId += 1;
        const sourceTriangle = Math.floor((offset - group.start) / 3);
        const diagnosticTriangle = Math.floor((sourceVertexIndices.length - groupVertexStart) / 3);
        const color = encodeOwnerId(id);
        const linear = ownerColorToLinearRgb(color);
          const materialIndex = group.materialIndex ?? 0;
        const material = materialAt(mesh, materialIndex);
        const indices = sourceIndex
          ? [sourceIndex.getX(offset), sourceIndex.getX(offset + 1), sourceIndex.getX(offset + 2)]
          : [offset, offset + 1, offset + 2];
        for (const index of indices) {
          sourceVertexIndices.push(index);
          colors.push(linear[0], linear[1], linear[2]);
        }
        const projection = ownerTriangleProjection(mesh, indices, viewProjection, target, clip);
        const frontFace = threeFrontFace(mesh, material);
        const gpuFrontFacing = projection
          ? gpuFrontFacingForFrontFace(projection.screenSignedArea, frontFace)
          : null;
        const record = {
          id,
          drawIndex: ownerIdRecords.length,
          color,
          meshName: mesh.name ?? '',
          meshUuid: mesh.uuid,
          materialIndex,
          materialSlot: materialIndex,
          materialName: material?.name ?? '',
          pass: materialPass(material),
          materialType: material?.type ?? null,
          side: material?.side ?? null,
          matrixWorldDeterminant: mesh.matrixWorld.determinant(),
          frontFace,
          cullMode: threeCullMode(material),
          transparent: material?.transparent ?? false,
          opacity: material?.opacity ?? 1.0,
          alphaTest: material?.alphaTest ?? 0.0,
          depthWrite: material?.depthWrite ?? true,
          depthTest: material?.depthTest ?? true,
          blending: material?.blending ?? null,
          premultipliedAlpha: material?.premultipliedAlpha ?? false,
          ownerColorSource: 'vertex-color',
          renderOrder: mesh.renderOrder ?? 0,
          renderPhaseOrder: material?.type === 'ShaderMaterial' ? (mesh.renderOrder ?? 0) : null,
          triangle: sourceTriangle,
          sourceTriangle,
          sourceGeometryTriangle: Math.floor(offset / 3),
          diagnosticTriangle,
          indices,
          screen: projection?.screen ?? null,
          screenBounds: projection?.screenBounds ?? null,
          depth: projection?.depth ?? null,
          webglDepth: projection?.webglDepth ?? null,
          depthRange: projection?.depthRange ?? null,
          screenSignedArea: projection?.screenSignedArea ?? null,
          frontFacing: projection?.frontFacing ?? null,
          gpuFrontFacing,
          visibleByCullPolicy: projection
            ? visibleByThreeCullPolicy(mesh, material, projection.screenSignedArea)
            : null,
        };
        ownerIdRecords.push(record);
        ownerIdByColor.set(ownerColorKey(color), record);
        ownerIdByCandidate.set(ownerCandidateKey(mesh.uuid, materialIndex, record.triangle), record);
        ownerIdByDiagnosticCandidate.set(
          ownerCandidateKey(mesh.uuid, materialIndex, diagnosticTriangle),
          record,
        );
      }
      diagnosticGeometry.addGroup(
        groupVertexStart,
        sourceVertexIndices.length - groupVertexStart,
        group.materialIndex ?? 0,
      );
    }
    for (const [name, attribute] of Object.entries(sourceGeometry.attributes)) {
      diagnosticGeometry.setAttribute(name, duplicateAttribute(attribute, sourceVertexIndices));
    }
    for (const [name, morphAttributes] of Object.entries(sourceGeometry.morphAttributes)) {
      diagnosticGeometry.morphAttributes[name] = morphAttributes.map((attribute) => (
        duplicateAttribute(attribute, sourceVertexIndices)
      ));
    }
    diagnosticGeometry.morphTargetsRelative = sourceGeometry.morphTargetsRelative;
    diagnosticGeometry.setAttribute('color', new THREE.BufferAttribute(new Float32Array(colors), 3));
    return diagnosticGeometry;
  };

  const ownerIdForCandidate = (mesh, materialIndex, triangle) => {
    const key = ownerCandidateKey(mesh.uuid, materialIndex, triangle);
    const record = ownerIdByDiagnosticCandidate.get(key) ?? ownerIdByCandidate.get(key);
    return record ? { id: record.id, color: record.color } : null;
  };

  const uvAttributeForMaterial = (geometry, material) => {
    const channel = material?.map?.channel ?? 0;
    return geometry.attributes[channel === 0 ? 'uv' : \`uv\${channel}\`] ?? geometry.attributes.uv ?? null;
  };

  const renderedHotspotOwner = (hotspot, renderedRgba) => {
    if (!renderedRgba || !Number.isInteger(hotspot?.x) || !Number.isInteger(hotspot?.y)) return null;
    const index = (hotspot.y * ${options.width} + hotspot.x) * 4;
    const color = [
      renderedRgba[index] ?? 0,
      renderedRgba[index + 1] ?? 0,
      renderedRgba[index + 2] ?? 0,
      renderedRgba[index + 3] ?? 0,
    ];
    const id = color[0] | (color[1] << 8) | (color[2] << 16);
    if (id === 0) return { id: null, color, owner: null };
    return {
      id,
      color,
      owner: ownerIdByColor.get(ownerColorKey(color)) ?? null,
    };
  };

  const ownerMatches = (owner, candidate) => (
    owner?.id != null && candidate?.ownerId != null && owner.id === candidate.ownerId
  );

  const compactOwnerCandidate = (candidate) => candidate ? {
    meshName: candidate.meshName,
    meshUuid: candidate.meshUuid,
    materialIndex: candidate.materialIndex,
    materialName: candidate.materialName,
    materialType: candidate.materialType,
    triangle: candidate.triangle,
    indices: candidate.indices,
    ownerId: candidate.ownerId,
    ownerIdColor: candidate.ownerIdColor,
    depth: candidate.depth,
    rawUv: candidate.rawUv,
    mapUv: candidate.mapUv,
    projectedBaseColorSrgb: candidate.projectedBaseColorSrgb,
  } : null;

  const summarizeProjectedHotspots = (hotspots) => {
    const summary = {
      total: hotspots.length,
      renderedOwnerCount: 0,
      renderedOwnerCandidateCount: 0,
      renderedOwnerFrontmostCount: 0,
      renderedOwnerBestSubpixelCount: 0,
      renderedOwnerBestSubpixelFrontmostCount: 0,
      renderedOwnerBestCoverageCount: 0,
      renderedOwnerBestCoverageFrontmostCount: 0,
      renderedOwnerBestNeighborCount: 0,
      renderedOwnerBestNeighborFrontmostCount: 0,
      renderedOwnerDepthRanks: {},
      renderedOwnerBestSubpixelDepthRanks: {},
      renderedOwnerBestCoverageDepthRanks: {},
      renderedOwnerBestNeighborDepthRanks: {},
      renderedOwnerBestSubpixelCenters: {},
      renderedOwnerBestCoverageCenters: {},
      renderedOwnerBestNeighborOffsets: {},
    };
    const bump = (map, key) => {
      map[key] = (map[key] ?? 0) + 1;
    };
    const sampleCenterKey = (sampleCenter) => sampleCenter
      .map((value) => Number(value.toFixed(6)).toString())
      .join(',');
    for (const hotspot of hotspots) {
      if (hotspot.renderedOwner?.id != null) summary.renderedOwnerCount += 1;
      if (hotspot.renderedOwnerCandidate) summary.renderedOwnerCandidateCount += 1;
      if (hotspot.ownerMatch?.frontmost) summary.renderedOwnerFrontmostCount += 1;
      if (hotspot.renderedOwnerDepthRank != null) {
        bump(summary.renderedOwnerDepthRanks, String(hotspot.renderedOwnerDepthRank));
      }
      const bestSubpixel = hotspot.renderedOwnerRecovery?.bestSubpixel;
      if (bestSubpixel) {
        summary.renderedOwnerBestSubpixelCount += 1;
        if (bestSubpixel.frontmost) summary.renderedOwnerBestSubpixelFrontmostCount += 1;
        bump(summary.renderedOwnerBestSubpixelDepthRanks, String(bestSubpixel.depthRank));
        bump(summary.renderedOwnerBestSubpixelCenters, sampleCenterKey(bestSubpixel.sampleCenter));
      }
      const bestCoverage = hotspot.renderedOwnerRecovery?.bestCoverage;
      if (bestCoverage) {
        summary.renderedOwnerBestCoverageCount += 1;
        if (bestCoverage.frontmost) summary.renderedOwnerBestCoverageFrontmostCount += 1;
        bump(summary.renderedOwnerBestCoverageDepthRanks, String(bestCoverage.depthRank));
        bump(summary.renderedOwnerBestCoverageCenters, sampleCenterKey(bestCoverage.sampleCenter));
      }
      const bestNeighbor = hotspot.renderedOwnerRecovery?.bestNeighbor;
      if (bestNeighbor) {
        summary.renderedOwnerBestNeighborCount += 1;
        if (bestNeighbor.frontmost) summary.renderedOwnerBestNeighborFrontmostCount += 1;
        bump(summary.renderedOwnerBestNeighborDepthRanks, String(bestNeighbor.depthRank));
        bump(summary.renderedOwnerBestNeighborOffsets, bestNeighbor.pixelOffset.join(','));
      }
    }
    return summary;
  };

  const projectHotspots = (root, camera, hotspots, sampleCenter, renderedRgba = null) => {
    if (!hotspots) return null;
    root.updateMatrixWorld(true);
    camera.updateMatrixWorld(true);
    const viewProjection = new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    const meshes = [];
    root.traverse((object) => {
      if (object.isMesh && object.geometry?.attributes?.position) meshes.push(object);
    });
    const vertex = new THREE.Vector3();
    const clip = new THREE.Vector4();
    const projectedTriangles = [];
    for (const mesh of meshes) {
      const geometry = mesh.geometry;
      const position = geometry.attributes.position;
      const index = geometry.index;
      const groups = geometry.groups.length > 0
        ? geometry.groups
        : [{ start: 0, count: index ? index.count : position.count, materialIndex: 0 }];
      for (const group of groups) {
        const material = materialAt(mesh, group.materialIndex);
        const uvAttribute = uvAttributeForMaterial(geometry, material);
        for (let offset = group.start; offset + 2 < group.start + group.count; offset += 3) {
          const ia = index ? index.getX(offset) : offset;
          const ib = index ? index.getX(offset + 1) : offset + 1;
          const ic = index ? index.getX(offset + 2) : offset + 2;
          const a = screenVertex(mesh, ia, uvAttribute, viewProjection, vertex, clip);
          const b = screenVertex(mesh, ib, uvAttribute, viewProjection, vertex, clip);
          const c = screenVertex(mesh, ic, uvAttribute, viewProjection, vertex, clip);
          if (!a || !b || !c) continue;
          const signedArea = (b.screen[0] - a.screen[0]) * (c.screen[1] - a.screen[1]) - (b.screen[1] - a.screen[1]) * (c.screen[0] - a.screen[0]);
          if (!visibleByThreeCullPolicy(mesh, material, signedArea)) continue;
        const materialIndex = group.materialIndex ?? 0;
          const triangle = Math.floor((offset - group.start) / 3);
          const owner = ownerIdForCandidate(mesh, materialIndex, triangle);
          projectedTriangles.push({
            meshName: mesh.name ?? '',
            meshUuid: mesh.uuid,
            materialIndex,
            materialName: material?.name ?? '',
            materialType: material?.type ?? null,
            material,
            triangle,
            indices: [ia, ib, ic],
            ownerId: owner?.id ?? null,
            ownerIdColor: owner?.color ?? null,
            a,
            b,
            c,
          });
        }
      }
    }

    const candidatesForPoint = (hotspot, point) => {
      const candidates = [];
      for (const projected of projectedTriangles) {
        const weights = barycentric(point, projected.a.screen, projected.b.screen, projected.c.screen);
        if (!weights) continue;
        const depth = weights[0] * projected.a.depth + weights[1] * projected.b.depth + weights[2] * projected.c.depth;
        if (depth < -1.0 || depth > 1.0) continue;
        const rawUv = interpolateUv(weights, projected.a, projected.b, projected.c);
        const mapUv = transformTextureUv(rawUv, projected.material?.map);
        const color = [
          quantize(linearToSrgb(mapUv[0])),
          quantize(linearToSrgb(mapUv[1])),
          0,
          hotspot.expected?.[3] ?? 255,
        ];
        const baseColor = projectedBaseColor(projected.material, mapUv, hotspot.expected?.[3] ?? 255);
        candidates.push({
          meshName: projected.meshName,
          meshUuid: projected.meshUuid,
          materialIndex: projected.materialIndex,
          materialName: projected.materialName,
          materialType: projected.materialType,
          triangle: projected.triangle,
          indices: projected.indices,
          ownerId: projected.ownerId,
          ownerIdColor: projected.ownerIdColor,
          depth,
          barycentric: weights,
          rawUv,
          mapUv,
          color,
          sampledMapRgba: baseColor.sampledMapRgba,
          projectedBaseColorSrgb: baseColor.projectedBaseColorSrgb,
          expectedRgbDistance: rgbDistance(color, hotspot.expected ?? [0, 0, 0, 255]),
          actualRgbDistance: rgbDistance(color, hotspot.actual ?? [0, 0, 0, 255]),
          projectedBaseColorExpectedRgbDistance: rgbDistance(baseColor.projectedBaseColorSrgb, hotspot.expected ?? [0, 0, 0, 255]),
          projectedBaseColorActualRgbDistance: rgbDistance(baseColor.projectedBaseColorSrgb, hotspot.actual ?? [0, 0, 0, 255]),
          screen: [projected.a.screen, projected.b.screen, projected.c.screen],
        });
      }
      return candidates;
    };

    const ownerMatchAtPoint = (hotspot, point, renderedOwner) => {
      if (renderedOwner?.id == null) return null;
      const candidates = candidatesForPoint(hotspot, point);
      const candidatesByDepth = candidates
        .filter((candidate) => candidate.depth >= -1.0 && candidate.depth <= 1.0)
        .sort((left, right) => left.depth - right.depth);
      const index = candidatesByDepth.findIndex((candidate) => candidate.ownerId === renderedOwner.id);
      if (index < 0) return null;
      const candidate = candidatesByDepth[index];
      return {
        sampleCenter: [point[0] - hotspot.x, point[1] - hotspot.y],
        depthRank: index + 1,
        frontmost: index === 0,
        depthDeltaFromFrontmost: candidatesByDepth[0] ? candidate.depth - candidatesByDepth[0].depth : null,
        candidate: compactOwnerCandidate(candidate),
      };
    };

    const ownerRecovery = (hotspot, renderedOwner) => {
      if (renderedOwner?.id == null || ownerIdRecords.length === 0) return null;
      const subpixelMatches = [];
      for (let row = 0; row < hotspotSubpixelSteps; row += 1) {
        const y = (row + 0.5) / hotspotSubpixelSteps;
        for (let column = 0; column < hotspotSubpixelSteps; column += 1) {
          const x = (column + 0.5) / hotspotSubpixelSteps;
          const match = ownerMatchAtPoint(hotspot, [hotspot.x + x, hotspot.y + y], renderedOwner);
          if (match) subpixelMatches.push(match);
        }
      }
      const coverageMatches = [];
      for (const projected of projectedTriangles) {
        if (projected.ownerId !== renderedOwner.id) continue;
        const coverage = pixelTriangleIntersectionPoint(hotspot.x, hotspot.y, projected);
        if (!coverage) continue;
        const match = ownerMatchAtPoint(hotspot, coverage.point, renderedOwner);
        if (match) {
          match.coverageBarycentric = coverage.barycentric;
          match.coveragePointCount = coverage.pointCount;
          match.coverageAreaPixels = coverage.areaPixels;
          coverageMatches.push(match);
        }
      }
      const neighborMatches = [];
      for (const dy of [-1, 0, 1]) {
        for (const dx of [-1, 0, 1]) {
          const match = ownerMatchAtPoint(hotspot, [hotspot.x + dx + 0.5, hotspot.y + dy + 0.5], renderedOwner);
          if (match) {
            match.pixelOffset = [dx, dy];
            neighborMatches.push(match);
          }
        }
      }
      return {
        subpixelSteps: hotspotSubpixelSteps,
        subpixelMatches,
        bestSubpixel: subpixelMatches.slice().sort((left, right) => left.depthRank - right.depthRank || Math.abs(left.sampleCenter[0] - 0.5) + Math.abs(left.sampleCenter[1] - 0.5) - (Math.abs(right.sampleCenter[0] - 0.5) + Math.abs(right.sampleCenter[1] - 0.5)))[0] ?? null,
        coverageMatches,
        bestCoverage: coverageMatches.slice().sort((left, right) => left.depthRank - right.depthRank || Math.abs(left.sampleCenter[0] - 0.5) + Math.abs(left.sampleCenter[1] - 0.5) - (Math.abs(right.sampleCenter[0] - 0.5) + Math.abs(right.sampleCenter[1] - 0.5)))[0] ?? null,
        neighborMatches,
        bestNeighbor: neighborMatches.slice().sort((left, right) => left.depthRank - right.depthRank || Math.abs(left.pixelOffset[0]) + Math.abs(left.pixelOffset[1]) - (Math.abs(right.pixelOffset[0]) + Math.abs(right.pixelOffset[1])))[0] ?? null,
      };
    };

    const top = hotspots.top.map((hotspot) => {
      const point = [hotspot.x + sampleCenter[0], hotspot.y + sampleCenter[1]];
      const candidates = candidatesForPoint(hotspot, point);
      const candidatesByDepth = candidates
          .filter((candidate) => candidate.depth >= -1.0 && candidate.depth <= 1.0)
          .sort((left, right) => left.depth - right.depth);
        const frontmost = candidatesByDepth[0] ?? null;
        const nearestExpected = candidates
          .slice()
          .sort((left, right) => left.expectedRgbDistance - right.expectedRgbDistance || left.depth - right.depth)[0] ?? null;
        const nearestActual = candidates
          .slice()
          .sort((left, right) => left.actualRgbDistance - right.actualRgbDistance || left.depth - right.depth)[0] ?? null;
        const nearestExpectedBaseColor = candidates
          .slice()
          .sort((left, right) => left.projectedBaseColorExpectedRgbDistance - right.projectedBaseColorExpectedRgbDistance || left.depth - right.depth)[0] ?? null;
        const nearestActualBaseColor = candidates
          .slice()
          .sort((left, right) => left.projectedBaseColorActualRgbDistance - right.projectedBaseColorActualRgbDistance || left.depth - right.depth)[0] ?? null;
        const renderedOwner = renderedHotspotOwner(hotspot, renderedRgba);
        const renderedOwnerCandidate = renderedOwner?.id == null
          ? null
          : candidates.find((candidate) => candidate.ownerId === renderedOwner.id) ?? null;
        const renderedOwnerDepthRank = renderedOwnerCandidate
          ? candidatesByDepth.findIndex((candidate) => candidate.ownerId === renderedOwnerCandidate.ownerId) + 1
          : null;
        const renderedOwnerRecovery = ownerRecovery(hotspot, renderedOwner);
        return {
          x: hotspot.x,
          y: hotspot.y,
          expected: hotspot.expected,
          actual: hotspot.actual,
          renderedOwner,
          frontmost,
          nearestExpected,
          nearestActual,
          nearestExpectedBaseColor,
          nearestActualBaseColor,
          renderedOwnerCandidate,
          renderedOwnerDepthRank: renderedOwnerDepthRank && renderedOwnerDepthRank > 0 ? renderedOwnerDepthRank : null,
          renderedOwnerDepthDeltaFromFrontmost: renderedOwnerCandidate && frontmost ? renderedOwnerCandidate.depth - frontmost.depth : null,
          renderedOwnerRecovery,
          ownerMatch: renderedOwner ? {
            frontmost: ownerMatches(renderedOwner, frontmost),
            nearestExpected: ownerMatches(renderedOwner, nearestExpected),
            nearestActual: ownerMatches(renderedOwner, nearestActual),
            nearestExpectedBaseColor: ownerMatches(renderedOwner, nearestExpectedBaseColor),
            nearestActualBaseColor: ownerMatches(renderedOwner, nearestActualBaseColor),
          } : null,
          candidateCount: candidates.length,
          candidatesByExpected: candidates
            .slice()
            .sort((left, right) => left.expectedRgbDistance - right.expectedRgbDistance || left.depth - right.depth)
            .slice(0, 8),
          candidatesByExpectedBaseColor: candidates
            .slice()
            .sort((left, right) => left.projectedBaseColorExpectedRgbDistance - right.projectedBaseColorExpectedRgbDistance || left.depth - right.depth)
            .slice(0, 8),
        };
      });
    return {
      source: hotspots.source,
      width: hotspots.width,
      height: hotspots.height,
      sampleCenter,
      subpixelSteps: hotspotSubpixelSteps,
      projectedTriangleCount: projectedTriangles.length,
      summary: summarizeProjectedHotspots(top),
      top,
    };
  };

  globalThis.captureVrmFrame = async () => {
    const canvas = document.getElementById('canvas');
    const renderer = new THREE.WebGLRenderer({
      canvas,
      alpha: ${transparent},
      antialias: false,
      premultipliedAlpha: false,
      preserveDrawingBuffer: true,
    });
    renderer.setPixelRatio(1);
    renderer.setSize(${options.width}, ${options.height}, false);
    renderer.setClearColor(0x000000, ${clearAlpha});
    renderer.setClearAlpha(${clearAlpha});
    renderer.outputColorSpace = THREE.SRGBColorSpace;

    const camera = new THREE.PerspectiveCamera(30.0, ${options.width} / ${options.height}, 0.1, 20.0);
    camera.position.set(0.0, ${options.cameraY}, ${options.cameraZ});
    camera.lookAt(0.0, ${options.targetY}, 0.0);

    const scene = new THREE.Scene();
    const light = new THREE.DirectionalLight(
      new THREE.Color(${options.directionalR}, ${options.directionalG}, ${options.directionalB}),
      ${options.directionalIntensity},
    );
    light.position.set(${options.directionalX}, ${options.directionalY}, ${options.directionalZ}).normalize();
    scene.add(light);
    const ambient = new THREE.AmbientLight(0xffffff, ${options.ambientIntensity});
    scene.add(ambient);

    const loader = new GLTFLoader();
    loader.register((parser) => new VRMLoaderPlugin(parser));
    const bytes = await (await fetch('/fixture.vrm')).arrayBuffer();
    const gltf = await new Promise((resolve, reject) => loader.parse(bytes, '', resolve, reject));
    const vrm = gltf.userData.vrm;
    if (!vrm) throw new Error('fixture did not load as VRM');
    const uvDiagnosticTexture = new THREE.DataTexture(new Uint8Array([255, 255, 255, 255]), 1, 1);
    uvDiagnosticTexture.colorSpace = THREE.SRGBColorSpace;
    uvDiagnosticTexture.needsUpdate = true;
    const configureTextureNoMips = (texture) => {
      if (!texture?.isTexture) return;
      const nearest = texture.minFilter === THREE.NearestFilter ||
        texture.minFilter === THREE.NearestMipmapNearestFilter ||
        texture.minFilter === THREE.NearestMipmapLinearFilter;
      texture.generateMipmaps = false;
      texture.minFilter = nearest ? THREE.NearestFilter : THREE.LinearFilter;
      texture.needsUpdate = true;
    };
    const configureTextureNearest = (texture) => {
      if (!texture?.isTexture) return;
      texture.generateMipmaps = false;
      texture.magFilter = THREE.NearestFilter;
      texture.minFilter = THREE.NearestFilter;
      texture.needsUpdate = true;
    };
    const configureMaterialTextureSampling = (material) => {
      if (!material) return;
      for (const value of Object.values(material)) {
        if (${forceNearestTextures}) {
          configureTextureNearest(value);
        } else {
          configureTextureNoMips(value);
        }
      }
      for (const uniform of Object.values(material.uniforms ?? {})) {
        if (${forceNearestTextures}) {
          configureTextureNearest(uniform?.value);
        } else {
          configureTextureNoMips(uniform?.value);
        }
      }
    };
    const diagnosticMaterials = [];
    const diagnosticMeshes = [];
    const expressions = ${JSON.stringify(options.expressions)};
    if (expressions.length > 0 && !vrm.expressionManager) {
      throw new Error('render expressions were requested, but the VRM has no expressionManager');
    }
    for (const [name, weight] of expressions) {
      vrm.expressionManager.setValue(name, weight);
    }
    vrm.update?.(${options.mtoonTime});
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();
    vrm.scene.updateMatrixWorld(true);
    const ownerViewProjection = new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    vrm.scene.traverse((object) => {
      object.frustumCulled = false;
      if ((${options.disableTextureMips} || ${forceNearestTextures}) && object.material) {
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        for (const material of materials) configureMaterialTextureSampling(material);
      }
      if (${disableOutlines} && object.material) {
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        for (const material of materials) {
          if (material && 'outlineWidthFactor' in material) {
            material.outlineWidthFactor = 0.0;
            material.needsUpdate = true;
          }
        }
      }
      if (${disableNormalMaps} && object.material) {
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        for (const material of materials) {
          if (!material) continue;
          if ('normalMap' in material) material.normalMap = null;
          if (material.normalScale && typeof material.normalScale.set === 'function') {
            material.normalScale.set(0.0, 0.0);
          }
          if (material.uniforms?.normalMap) material.uniforms.normalMap.value = null;
          if (material.uniforms?.normalScale?.value?.set) {
            material.uniforms.normalScale.value.set(0.0, 0.0);
          }
          material.needsUpdate = true;
        }
      }
      if (${JSON.stringify(options.diagnosticRender)} !== 'shaded' && object.isMesh && object.material) {
        diagnosticMeshes.push(geometryReport(object));
        const mode = ${JSON.stringify(options.diagnosticRender)};
        if (mode === 'owner-id') {
          object.geometry = buildOwnerDiagnosticGeometry(object, ownerViewProjection);
        }
        const diagnosticMaterial = (material, mesh, slot) => {
          diagnosticMaterials.push(materialReport(material, mesh, slot));
          if (mode === 'owner-id') {
            const owner = new THREE.MeshBasicMaterial({
              color: 0xffffff,
              vertexColors: true,
              side: material?.side ?? THREE.FrontSide,
              transparent: material?.transparent ?? false,
              opacity: material?.opacity ?? 1.0,
              alphaTest: material?.alphaTest ?? 0.0,
              depthWrite: material?.depthWrite ?? true,
              depthTest: material?.depthTest ?? true,
            });
            owner.name = (material?.name ?? 'material') + ':vrm-rs-owner-id-diagnostic';
            owner.blending = material?.blending ?? THREE.NormalBlending;
            owner.premultipliedAlpha = material?.premultipliedAlpha ?? false;
            return owner;
          }
          if (mode === 'uv' || mode === 'base-uv') {
            const uv = new THREE.MeshBasicMaterial({
              color: 0xffffff,
              map: mode === 'base-uv' ? material?.map ?? uvDiagnosticTexture : uvDiagnosticTexture,
              side: material?.side ?? THREE.FrontSide,
              transparent: material?.transparent ?? false,
              opacity: material?.opacity ?? 1.0,
              alphaTest: material?.alphaTest ?? 0.0,
              depthWrite: material?.depthWrite ?? true,
              depthTest: material?.depthTest ?? true,
            });
            uv.name = (material?.name ?? 'material') + ':vrm-rs-uv-diagnostic';
            uv.blending = material?.blending ?? THREE.NormalBlending;
            uv.premultipliedAlpha = material?.premultipliedAlpha ?? false;
            uv.onBeforeCompile = (shader) => {
              shader.fragmentShader = shader.fragmentShader.replace(
                '#include <map_fragment>',
                'diffuseColor = vec4(vMapUv, 0.0, diffuseColor.a);',
              );
            };
            return uv;
          }
          const color = (mode === 'base-factor' || mode === 'base-color' || mode === 'base-color-flip-v' || mode === 'base-color-raw-srgb') && material?.color?.isColor === true
            ? material.color.clone()
            : new THREE.Color(0xffffff);
          const flat = new THREE.MeshBasicMaterial({
            color,
            map: mode === 'base-color' || mode === 'base-color-flip-v' || mode === 'base-color-raw-srgb' ? material?.map ?? null : null,
            side: material?.side ?? THREE.FrontSide,
            transparent: material?.transparent ?? false,
            opacity: material?.opacity ?? 1.0,
            alphaTest: material?.alphaTest ?? 0.0,
            depthWrite: material?.depthWrite ?? true,
            depthTest: material?.depthTest ?? true,
          });
          flat.name = (material?.name ?? 'material') + ':vrm-rs-flat-diagnostic';
          flat.blending = material?.blending ?? THREE.NormalBlending;
          flat.premultipliedAlpha = material?.premultipliedAlpha ?? false;
          return flat;
        };
        object.material = Array.isArray(object.material)
          ? object.material.map((material, slot) => diagnosticMaterial(material, object, slot))
          : diagnosticMaterial(object.material, object, 0);
      }
    });
    scene.add(vrm.scene);
    renderer.clear(true, true, true);
    renderer.render(scene, camera);

    const gl = renderer.getContext();
    const readback = new Uint8Array(${options.width} * ${options.height} * 4);
    gl.readPixels(0, 0, ${options.width}, ${options.height}, gl.RGBA, gl.UNSIGNED_BYTE, readback);
    const rgba = new Uint8Array(readback.length);
    const rowBytes = ${options.width} * 4;
    for (let y = 0; y < ${options.height}; y += 1) {
      const source = (${options.height} - 1 - y) * rowBytes;
      const destination = y * rowBytes;
      rgba.set(readback.subarray(source, source + rowBytes), destination);
    }
    const diagnosticHotspots = projectHotspots(vrm.scene, camera, hotspotDeltas, hotspotSampleCenter, rgba);
    let imqraw = null;
    if (${options.imqraw}) {
      await initImqraw();
      imqraw = Array.from(encodeRgba8(rgba, ${options.width}, ${options.height}, {
        label: 'three-vrm',
        tags: ['three-vrm', 'reference'],
      }));
    }
    const reference = {
      threeRevision: THREE.REVISION,
      renderer: {
        outputColorSpace: renderer.outputColorSpace,
        toneMapping: renderer.toneMapping,
        toneMappingExposure: renderer.toneMappingExposure,
        alpha: ${transparent},
        clearAlpha: ${clearAlpha},
        antialias: false,
        premultipliedAlpha: false,
        disableOutlines: ${disableOutlines},
        disableNormalMaps: ${disableNormalMaps},
        disableTextureMips: ${options.disableTextureMips},
        forceNearestTextures: ${forceNearestTextures},
        diagnosticRender: ${JSON.stringify(options.diagnosticRender)},
        diagnosticRenderReference: ${JSON.stringify(options.diagnosticRender === 'base-color-flip-v' || options.diagnosticRender === 'base-color-raw-srgb' ? 'base-color' : options.diagnosticRender)},
        rustOnlyDiagnostic: ${JSON.stringify(options.diagnosticRender === 'base-color-flip-v' || options.diagnosticRender === 'base-color-raw-srgb' ? options.diagnosticRender : null)},
        diagnosticMaterials,
        diagnosticMeshes,
        diagnosticOwnerIds: ownerIdRecords,
        diagnosticHotspots,
      },
      expressions,
      lighting: {
        directional: {
          color: [${options.directionalR}, ${options.directionalG}, ${options.directionalB}],
          intensity: ${options.directionalIntensity},
          position: light.position.toArray(),
        },
        ambient: {
          color: '#ffffff',
          intensity: ambient.intensity,
        },
      },
      camera: {
        fov: camera.fov,
        aspect: camera.aspect,
        near: camera.near,
        far: camera.far,
        position: camera.position.toArray(),
        target: [0.0, ${options.targetY}, 0.0],
      },
    };
    renderer.dispose();
    return { rgba: Array.from(rgba), imqraw, reference };
  };
</script>`;
}

function parseExpressionWeights(values) {
  return values.map((value) => {
    const separator = value.indexOf('=');
    if (separator <= 0 || separator === value.length - 1) {
      console.error(`invalid expression '${value}', expected name=weight`);
      process.exit(2);
    }
    const name = value.slice(0, separator);
    const weight = Number(value.slice(separator + 1));
    if (!Number.isFinite(weight)) {
      console.error(`invalid expression weight in '${value}'`);
      process.exit(2);
    }
    return [name, weight];
  });
}

function readHotspotDeltas(file, top) {
  const resolved = path.resolve(file);
  if (!fs.existsSync(resolved)) {
    console.error(`hotspot delta report does not exist: ${resolved}`);
    process.exit(2);
  }
  const parsed = JSON.parse(fs.readFileSync(resolved, 'utf8'));
  if (!Array.isArray(parsed.top)) {
    console.error(`hotspot delta report has no top array: ${resolved}`);
    process.exit(2);
  }
  return {
    source: resolved,
    width: parsed.width,
    height: parsed.height,
    top: parsed.top.slice(0, top),
  };
}

function encodePngRgba(width, height, rgba) {
  if (rgba.length !== width * height * 4) {
    throw new Error(`rgba length ${rgba.length} does not match ${width}x${height}`);
  }

  const scanlineBytes = width * 4;
  const raw = Buffer.alloc((scanlineBytes + 1) * height);
  const rgbaBytes = Buffer.from(rgba);
  for (let y = 0; y < height; y += 1) {
    const rawOffset = y * (scanlineBytes + 1);
    raw[rawOffset] = 0;
    rgbaBytes.copy(raw, rawOffset + 1, y * scanlineBytes, (y + 1) * scanlineBytes);
  }

  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  return Buffer.concat([
    signature,
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', zlib.deflateSync(raw)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

function pngChunk(type, data) {
  const typeBytes = Buffer.from(type, 'ascii');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBytes, data])), 0);
  return Buffer.concat([length, typeBytes, data, crc]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function contentType(file) {
  if (file.endsWith('.js')) return 'text/javascript; charset=utf-8';
  if (file.endsWith('.vrm')) return 'model/gltf-binary';
  return 'application/octet-stream';
}

function serveFile(response, file) {
  response.writeHead(200, { 'content-type': contentType(file) });
  fs.createReadStream(file).pipe(response);
}
