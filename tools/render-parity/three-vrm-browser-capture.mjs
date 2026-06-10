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
const diagnosticRender = args.get('diagnostic-render') ?? 'shaded';
const expressionWeights = parseExpressionWeights(expressions);

if (!fixture || !out) {
  console.error('usage: node tools/render-parity/three-vrm-browser-capture.mjs --fixture avatar.vrm --three-vrm-root ../three-vrm --out frame.rgba.json [--png-out frame.png] [--imqraw-out frame.imqraw] [--width 512] [--height 512] [--background opaque-black|transparent] [--ambient-intensity 0.1] [--directional-intensity PI] [--directional-r 1.0] [--expression happy=1.0] [--disable-outlines] [--disable-normal-maps] [--disable-texture-mips] [--diagnostic-render shaded|flat|base-factor|base-color|base-color-flip-v|uv]');
  process.exit(2);
}
if (![width, height].every((value) => Number.isInteger(value) && value > 0)) {
  console.error(`invalid dimensions: ${width}x${height}`);
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
if (!['shaded', 'flat', 'base-factor', 'base-color', 'base-color-flip-v', 'uv'].includes(diagnosticRender)) {
  console.error(`invalid diagnostic-render: ${diagnosticRender}; expected shaded, flat, base-factor, base-color, base-color-flip-v, or uv`);
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
    const configureMaterialNoMips = (material) => {
      if (!material) return;
      for (const value of Object.values(material)) {
        configureTextureNoMips(value);
      }
      for (const uniform of Object.values(material.uniforms ?? {})) {
        configureTextureNoMips(uniform?.value);
      }
    };
    vrm.scene.traverse((object) => {
      object.frustumCulled = false;
      if (${options.disableTextureMips} && object.material) {
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        for (const material of materials) configureMaterialNoMips(material);
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
        const diagnosticMaterial = (material, mesh) => {
          const mode = ${JSON.stringify(options.diagnosticRender)};
          if (mode === 'uv') {
            const uv = new THREE.MeshBasicMaterial({
              color: 0xffffff,
              map: uvDiagnosticTexture,
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
          const color = (mode === 'base-factor' || mode === 'base-color' || mode === 'base-color-flip-v') && material?.color?.isColor === true
            ? material.color.clone()
            : new THREE.Color(0xffffff);
          const flat = new THREE.MeshBasicMaterial({
            color,
            map: mode === 'base-color' || mode === 'base-color-flip-v' ? material?.map ?? null : null,
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
          ? object.material.map((material) => diagnosticMaterial(material, object))
          : diagnosticMaterial(object.material, object);
      }
    });
    scene.add(vrm.scene);
    const expressions = ${JSON.stringify(options.expressions)};
    if (expressions.length > 0 && !vrm.expressionManager) {
      throw new Error('render expressions were requested, but the VRM has no expressionManager');
    }
    for (const [name, weight] of expressions) {
      vrm.expressionManager.setValue(name, weight);
    }
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
        diagnosticRender: ${JSON.stringify(options.diagnosticRender)},
        diagnosticRenderReference: ${JSON.stringify(options.diagnosticRender === 'base-color-flip-v' ? 'base-color' : options.diagnosticRender)},
        rustOnlyDiagnostic: ${JSON.stringify(options.diagnosticRender === 'base-color-flip-v' ? 'base-color-flip-v' : null)},
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
