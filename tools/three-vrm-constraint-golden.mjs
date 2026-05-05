#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

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
const threeVrmRoot = args.get('three-vrm-root') ?? '../three-vrm';
const out = args.get('out');

if (!fixture) {
  console.error('usage: node tools/three-vrm-constraint-golden.mjs --fixture path/to/avatar.vrm [--three-vrm-root ../three-vrm] [--out path.json]');
  process.exit(2);
}

globalThis.self = globalThis;
globalThis.createImageBitmap = async () => ({ close() {} });

const root = path.resolve(threeVrmRoot);
const threePackage = path.join(root, 'packages/three-vrm');
const [{ GLTFLoader }, { VRMLoaderPlugin }, THREE] = await Promise.all([
  import(pathToFileURL(path.join(threePackage, 'node_modules/three/examples/jsm/loaders/GLTFLoader.js')).href),
  import(pathToFileURL(path.join(threePackage, 'lib/three-vrm.module.js')).href),
  import(pathToFileURL(path.join(threePackage, 'node_modules/three/build/three.module.js')).href),
]);

const loader = new GLTFLoader();
loader.register((parser) => new VRMLoaderPlugin(parser));

const bytes = fs.readFileSync(fixture);
const arrayBuffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
const gltf = await new Promise((resolve, reject) => loader.parse(arrayBuffer, '', resolve, reject));
const vrm = gltf.userData.vrm;
const manager = vrm?.nodeConstraintManager;
if (!manager || manager.constraints.size === 0) {
  console.error(`fixture has no three-vrm nodeConstraintManager constraints: ${fixture}`);
  process.exit(3);
}

const associationFor = (object) => gltf.parser.associations.get(object)?.nodes ?? null;
const rounded = (value) => Number(value.toFixed(8));
const quaternion = (value) => [value.x, value.y, value.z, value.w].map(rounded);
const eulerQuaternion = (index) => new THREE.Quaternion().setFromEuler(
  new THREE.Euler(0.02 + index * 0.011, -0.015 + index * 0.007, 0.01 - index * 0.005, 'XYZ'),
);

const constraints = [...manager.constraints];
const sourceInputs = new Map();
for (const [index, constraint] of constraints.entries()) {
  const sourceNode = associationFor(constraint.source);
  if (sourceNode == null || sourceInputs.has(sourceNode)) continue;
  sourceInputs.set(sourceNode, eulerQuaternion(index));
}

manager.setInitState();
for (const [node, rotation] of sourceInputs) {
  const object = constraints.find((constraint) => associationFor(constraint.source) === node).source;
  object.quaternion.copy(rotation);
  object.updateMatrix();
}
gltf.scene.updateMatrixWorld(true);

const updateOrder = [];
for (const constraint of constraints) {
  const original = constraint.update.bind(constraint);
  constraint.update = () => {
    updateOrder.push(associationFor(constraint.destination));
    original();
  };
}

manager.update();

const result = {
  generator: 'vrm-rs tools/three-vrm-constraint-golden.mjs',
  fixture: path.resolve(fixture),
  constraints: constraints.map((constraint, index) => ({
    index,
    destination: associationFor(constraint.destination),
    source: associationFor(constraint.source),
    weight: rounded(constraint.weight),
    localRotation: quaternion(constraint.destination.quaternion),
  })),
  sourceInputs: [...sourceInputs.entries()].map(([node, rotation]) => ({
    node,
    localRotation: quaternion(rotation),
  })),
  updateOrder,
};

const json = `${JSON.stringify(result, null, 2)}\n`;
if (out) {
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, json);
} else {
  process.stdout.write(json);
}
