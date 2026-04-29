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
const delta = Number(args.get('delta') ?? '0.016666666666666666');

if (!fixture) {
  console.error('usage: node tools/three-vrm-golden.mjs --fixture path/to/avatar.vrm [--three-vrm-root ../three-vrm] [--delta 0.0166667] [--out path.json]');
  process.exit(2);
}

globalThis.self = globalThis;
globalThis.createImageBitmap = async () => ({ close() {} });

const root = path.resolve(threeVrmRoot);
const threePackage = path.join(root, 'packages/three-vrm');
const [{ GLTFLoader }, { VRMLoaderPlugin }] = await Promise.all([
  import(pathToFileURL(path.join(threePackage, 'node_modules/three/examples/jsm/loaders/GLTFLoader.js')).href),
  import(pathToFileURL(path.join(threePackage, 'lib/three-vrm.module.js')).href),
]);

const loader = new GLTFLoader();
loader.register((parser) => new VRMLoaderPlugin(parser));

const bytes = fs.readFileSync(fixture);
const arrayBuffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
const gltf = await new Promise((resolve, reject) => loader.parse(arrayBuffer, '', resolve, reject));
const vrm = gltf.userData.vrm;
if (!vrm?.springBoneManager) {
  console.error(`fixture has no three-vrm springBoneManager: ${fixture}`);
  process.exit(3);
}

vrm.springBoneManager.setInitState();
vrm.springBoneManager.update(delta);

const associationFor = (object) => gltf.parser.associations.get(object)?.nodes ?? null;
const rounded = (value) => Number(value.toFixed(8));
const vector = (value) => value.toArray().map(rounded);
const quaternion = (value) => [value.x, value.y, value.z, value.w].map(rounded);

const springJoints = [...vrm.springBoneManager.joints]
  .map((joint, index) => ({
    index,
    node: associationFor(joint.bone),
    name: joint.bone.name,
    childNode: joint.child ? associationFor(joint.child) : null,
    childName: joint.child?.name ?? null,
    initialLocalChildPosition: vector(joint.initialLocalChildPosition),
    localRotation: quaternion(joint.bone.quaternion),
  }))
  .filter((joint) => joint.node != null);

const result = {
  generator: 'vrm-rs tools/three-vrm-golden.mjs',
  threeVrmVersion: vrm.constructor?.name ?? 'VRM',
  fixture: path.resolve(fixture),
  delta,
  springJoints,
};

const json = `${JSON.stringify(result, null, 2)}\n`;
if (out) {
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, json);
} else {
  process.stdout.write(json);
}
