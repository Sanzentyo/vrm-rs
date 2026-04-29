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
const vrma = args.get('vrma');
const threeVrmRoot = args.get('three-vrm-root') ?? '../three-vrm';
const out = args.get('out');
const times = (args.get('times') ?? '0,0.5,1.0')
  .split(',')
  .map((value) => Number(value.trim()))
  .filter((value) => Number.isFinite(value) && value >= 0.0);

if (!fixture || !vrma) {
  console.error('usage: node tools/three-vrm-vrma-golden.mjs --fixture path/to/avatar.vrm --vrma path/to/clip.vrma [--three-vrm-root ../three-vrm] [--times 0,0.5,1.0] [--out path.json]');
  process.exit(2);
}
if (times.length === 0) {
  console.error(`invalid --times: ${args.get('times')}`);
  process.exit(2);
}

globalThis.self = globalThis;
globalThis.createImageBitmap = async () => ({ close() {} });

const root = path.resolve(threeVrmRoot);
const threePackage = path.join(root, 'packages/three-vrm');
const animationPackage = path.join(root, 'packages/three-vrm-animation');
const [{ GLTFLoader }, { VRMLoaderPlugin }, animationModule, THREE] = await Promise.all([
  import(pathToFileURL(path.join(threePackage, 'node_modules/three/examples/jsm/loaders/GLTFLoader.js')).href),
  import(pathToFileURL(path.join(threePackage, 'lib/three-vrm.module.js')).href),
  import(pathToFileURL(path.join(animationPackage, 'lib/three-vrm-animation.module.js')).href),
  import(pathToFileURL(path.join(threePackage, 'node_modules/three/build/three.module.js')).href),
]);
const { VRMAnimationLoaderPlugin, VRMLookAtQuaternionProxy, createVRMAnimationClip } = animationModule;

const rounded = (value) => Number(value.toFixed(8));
const quaternion = (value) => [value.x, value.y, value.z, value.w].map(rounded);
const pose = (value) => Object.fromEntries(Object.entries(value ?? {}).map(([bone, transform]) => [
  bone,
  {
    position: transform.position?.map(rounded) ?? null,
    rotation: transform.rotation?.map(rounded) ?? null,
  },
]));

const loadVrm = async (file) => {
  const loader = new GLTFLoader();
  loader.register((parser) => new VRMLoaderPlugin(parser));
  const bytes = fs.readFileSync(file);
  const arrayBuffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  const gltf = await new Promise((resolve, reject) => loader.parse(arrayBuffer, '', resolve, reject));
  const loaded = gltf.userData.vrm;
  if (!loaded) throw new Error(`fixture did not load as VRM: ${file}`);
  return loaded;
};

const loadVrma = async (file) => {
  const loader = new GLTFLoader();
  loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
  const bytes = fs.readFileSync(file);
  const arrayBuffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  const gltf = await new Promise((resolve, reject) => loader.parse(arrayBuffer, '', resolve, reject));
  const animations = gltf.userData.vrmAnimations ?? [];
  if (animations.length === 0) throw new Error(`fixture did not load as VRMA: ${file}`);
  return animations[0];
};

const expressionWeights = (manager) => {
  if (!manager) return {};
  return Object.fromEntries(Object.keys(manager.expressionMap)
    .map((name) => [name, rounded(manager.getValue(name) ?? 0)]));
};

const vrm = await loadVrm(fixture);
const vrmAnimation = await loadVrma(vrma);
const lookAtProxy = vrm.lookAt ? new VRMLookAtQuaternionProxy(vrm.lookAt) : null;
if (lookAtProxy) {
  lookAtProxy.name = 'VRMLookAtQuaternionProxy';
  vrm.scene.add(lookAtProxy);
}
const clip = createVRMAnimationClip(vrmAnimation, vrm);
const mixer = new THREE.AnimationMixer(vrm.scene);
const action = mixer.clipAction(clip);
action.play();

const samples = times.map((time) => {
  vrm.humanoid.resetRawPose();
  vrm.humanoid.resetNormalizedPose();
  vrm.expressionManager?.resetValues();
  if (lookAtProxy) lookAtProxy.quaternion.identity();
  mixer.setTime(time);
  vrm.humanoid.update();
  vrm.expressionManager?.update();
  vrm.scene.updateMatrixWorld(true);
  return {
    time: rounded(time),
    rawAbsolutePose: pose(vrm.humanoid.getRawAbsolutePose()),
    normalizedPose: pose(vrm.humanoid.getNormalizedPose()),
    expressionWeights: expressionWeights(vrm.expressionManager),
    lookAtQuaternion: lookAtProxy ? quaternion(lookAtProxy.quaternion) : null,
    lookAtYawPitch: vrm.lookAt ? {
      yaw: rounded(vrm.lookAt.yaw),
      pitch: rounded(vrm.lookAt.pitch),
    } : null,
  };
});

const result = {
  generator: 'vrm-rs tools/three-vrm-vrma-golden.mjs',
  fixture: path.resolve(fixture),
  vrma: path.resolve(vrma),
  duration: rounded(vrmAnimation.duration),
  times: times.map(rounded),
  samples,
};

const json = `${JSON.stringify(result, null, 2)}\n`;
if (out) {
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, json);
} else {
  process.stdout.write(json);
}
