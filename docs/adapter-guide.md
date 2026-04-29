# Adapter Guide

`vrm-rs` does not require Bevy, wgpu, ash, or any specific engine.

Engines integrate by implementing traits from `vrm-adapter`:

- `SceneGraph`: parent/child traversal.
- `TransformAccess`: transform reads/writes.
- `WorldTransformAccess` / `WorldTransformUpdate`: world transform reads and explicit synchronization before parity spring simulation.
- `MorphTargetAccess`: expression morph target application.
- `MaterialAccess`: material color, texture transform, and HDR emissive intensity application.
- `MtoonPipelineAccess`: MToon base/outline pipeline pass selection.
- `TextureResolver`: texture handle lookup.
- `AnimationSink`: consume runtime events.

Engines that want humanoid pose parity can capture a `HumanoidPoseRig` from their scene graph. It provides raw absolute/rest-relative pose access, normalized absolute/rest-relative pose storage, and normalized-to-raw writeback through transform traits.

Engines that want a single orchestration point can use `VrmRuntimeDriver`. It applies animation frames, runtime expression events, node constraints, spring bone, first-person visibility, one-shot VRM0 root orientation compatibility, MToon pipeline hints, and effective emissive strength writeback through the traits above.

For spring bone parity with three-vrm, capture `SpringRestMap` once after the engine scene has its initial local/world transforms, then create a `CenterSpringRuntimeState` with `SpringRestMap::runtime_state`. Pass both to `VrmRuntimeDriver::tick_with_spring_parity` or call `step_spring_bone_system_parity` directly. This path keeps particle tails in center space, uses each selected child's local translation like three-vrm's `child.position`, uses sparse spring-chain child selection before falling back to the first scene child, and preserves the VRM0 7cm final-joint fallback. `tick_with_spring_parity` requires `WorldTransformUpdate` and synchronizes world transforms after animation/constraint writes and before spring simulation. The older `SpringRuntimeState` and `tick` remain available as a simpler world-space stepping path.

First-person `auto` annotations use `SceneGraph` plus the humanoid head bone to hide head descendants in first-person view while keeping them visible in third-person view. Engines that expose skinned mesh indices, skin weights, and skeleton joints can also implement `FirstPersonMeshAccess` and use the headless mesh planning helpers to clone first-person meshes with head-weighted triangles removed.

For Bevy, `vrm-adapter-bevy` starts with a 0.18.1 registry/descriptor/runtime skeleton. `BevyNodeMap` maps `NodeRef` to entities, `BevyRuntimeSceneState` is a lightweight `Entity` keyed scene state implementing the transform, world-transform, scene-graph, visibility, constraint-rest, morph target, material, and MToon pipeline traits, `bevy_mtoon_descriptors` bridges renderer-agnostic MToon descriptors, `bevy_mtoon_material_plans` converts those descriptors into Bevy-facing material plans with pass kind, render order, alpha/cull/depth state, colors, emissive intensity, cutoff, texture references, and outline width, and `VrmRuntimePlugin` installs `BevyVrmRuntimeConfig` as the first ECS entry point for runtime policy. The Bevy crate still avoids render features and shader policy; downstream apps can map material plans to `StandardMaterial`, custom MToon materials, or render-graph-specific assets. Concrete Bevy mesh/material asset mutation systems are still downstream work; the current scene state can be passed to `VrmRuntimeDriver` and records morph weights, material colors, texture transforms, emissive intensities, MToon passes, and visibility changes for tests and integration staging. For custom wgpu/ash engines, keep renderer resources outside `vrm-rs` and use `MaterialRef`/`TextureRef` as stable lookup keys into engine-owned tables.
