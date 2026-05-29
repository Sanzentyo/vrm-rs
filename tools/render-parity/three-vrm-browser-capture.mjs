#!/usr/bin/env node

import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import process from 'node:process';
import zlib from 'node:zlib';

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const arg = process.argv[index];
  if (!arg.startsWith('--')) continue;
  const key = arg.slice(2);
  const next = process.argv[index + 1];
  if (next == null || next.startsWith('--')) {
    args.set(key, 'true');
  } else {
    args.set(key, next);
    index += 1;
  }
}

const fixture = args.get('fixture');
const threeVrmRoot = path.resolve(args.get('three-vrm-root') ?? '../three-vrm');
const out = args.get('out');
const pngOut = args.get('png-out');
const width = Number.parseInt(args.get('width') ?? '512', 10);
const height = Number.parseInt(args.get('height') ?? '512', 10);
const cameraY = Number(args.get('camera-y') ?? '1.0');
const cameraZ = Number(args.get('camera-z') ?? '5.0');
const targetY = Number(args.get('target-y') ?? '1.0');
const mtoonTime = Number(args.get('mtoon-time') ?? '0.0');
const background = args.get('background') ?? 'opaque-black';

if (!fixture || !out) {
  console.error('usage: node tools/render-parity/three-vrm-browser-capture.mjs --fixture avatar.vrm --three-vrm-root ../three-vrm --out frame.rgba.json [--png-out frame.png] [--width 512] [--height 512] [--background opaque-black|transparent]');
  process.exit(2);
}
if (![width, height].every((value) => Number.isInteger(value) && value > 0)) {
  console.error(`invalid dimensions: ${width}x${height}`);
  process.exit(2);
}
if (![cameraY, cameraZ, targetY, mtoonTime].every(Number.isFinite)) {
  console.error('camera-y, camera-z, target-y, and mtoon-time must be finite numbers');
  process.exit(2);
}
if (!['opaque-black', 'transparent'].includes(background)) {
  console.error(`invalid background: ${background}; expected opaque-black or transparent`);
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
    response.end(capturePage({ width, height, cameraY, cameraZ, targetY, mtoonTime, background }));
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
    mtoonTime,
    format: 'rgba8',
    rgba: capture.rgba,
  }, null, 2)}\n`;
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, json);
  if (pngOut) {
    fs.mkdirSync(path.dirname(pngOut), { recursive: true });
    fs.writeFileSync(pngOut, encodePngRgba(width, height, capture.rgba));
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
    const light = new THREE.DirectionalLight(0xffffff, Math.PI);
    light.position.set(1.0, 1.0, 1.0).normalize();
    scene.add(light);
    scene.add(new THREE.AmbientLight(0xffffff, 0.1));

    const loader = new GLTFLoader();
    loader.register((parser) => new VRMLoaderPlugin(parser));
    const bytes = await (await fetch('/fixture.vrm')).arrayBuffer();
    const gltf = await new Promise((resolve, reject) => loader.parse(bytes, '', resolve, reject));
    const vrm = gltf.userData.vrm;
    if (!vrm) throw new Error('fixture did not load as VRM');
    vrm.scene.traverse((object) => {
      object.frustumCulled = false;
    });
    scene.add(vrm.scene);
    vrm.update?.(${options.mtoonTime});
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
    renderer.dispose();
    return { rgba: Array.from(rgba) };
  };
</script>`;
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
