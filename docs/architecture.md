# Architecture

## Crates

- `vrm-protocol`: serde wire types for VRM 0.0, VRMC_vrm 1.0, VRMC_springBone, VRMC_node_constraint, VRMC_materials_mtoon, KHR emissive strength, and VRMC_vrm_animation.
- `vrm-core`: pure domain types. This crate defines `VrmAsset<State>`, `VrmModel<State>`, `NodeRef`, `MaterialRef`, `TextureRef`, humanoid, expressions, lookAt, spring bone, constraints, material parameters, and animation tracks.
- `vrm-sans-io`: side-effect-free conversion from protocol data into validated core data.
- `vrm-io`: glTF/GLB IO through the `gltf` crate, extension extraction, rest scene graph extraction, buffer/image collection, and model construction.
- `vrm-runtime`: renderer-independent update orchestration and algorithms.
- `vrm-adapter`: traits for scene graph, transforms, morph targets, materials, textures, and animation sinks.
- `vrm-adapter-bevy`: Bevy 0.18.1 registry, descriptor bridge, and runtime plugin config skeleton.
- `vrm-rs`: facade crate. It re-exports the lower layers and provides `Vrm`, `load_full`, `load_runtime`, `runtime_for`, and `driver_for` so common applications can move from bytes/path loading to resolved documents, runtime events, and adapter drivers without importing every crate directly.

## Type State

The public lifecycle is:

```rust
VrmAsset<Parsed> -> VrmAsset<Validated> -> VrmModel<Resolved>
```

Only resolved models should be driven by runtime or attached to adapters. Raw node/material/texture references are represented with newtypes, not plain `usize`.

## Update Order

Runtime preserves the `three-vrm` order:

1. Humanoid
2. LookAt
3. Expressions
4. Node constraints
5. Spring bone
6. Materials

The current implementation produces deterministic runtime events and update orders. Engine-specific mutation belongs in adapters.

## Animation Extraction

VRMA files can contain multiple glTF animation clips. `vrm-io` classifies glTF animation channels by the `VRMC_vrm_animation` node map and stores extracted clips in `VrmDocument::animations`; `VrmDocument::animation` mirrors the first clip as a convenience value.

`LoadedVrm::scene` exposes `GltfSceneRest`, a renderer-independent snapshot of glTF node parent/child relationships plus local and world rest transforms. This keeps IO useful for adapter setup, fixture-driven parity tests, and custom engines that want a starting scene map without depending on Bevy, wgpu, ash, or three.js objects.

Current channel mapping:

- Humanoid `rotation` channels become per-bone `RotationTrack`s after rest-pose aware remapping.
- `hips` `translation` channels become `hips_translation` after mapping through the hips parent rest matrix.
- Expression `translation` channels use the X component as scalar expression weight.
- lookAt `rotation` channels become `look_at_track`.

`VrmAnimation::rest_hips_position` captures the hips node rest world position. `vrm-runtime` samples clips into `VrmAnimationFrame`, and `vrm-adapter` applies those frames through `TransformAccess`, `MorphTargetAccess`, and `MaterialAccess` without depending on Bevy, wgpu, ash, or glTF node objects.

## Runtime Solvers

Runtime math remains renderer-agnostic. Engine adapters are expected to provide current transforms and apply returned rotations/positions.

- Node constraints expose pure solvers for rotation, roll, and aim constraints.
- Spring bone exposes world-space particle state, typed center-space parity particle state, rest state, Verlet-style step helpers, center-space parity stepping, and sphere/capsule/plane collision correction including extended collider inside behavior.
- Adapter spring parity uses `SpringRestMap` plus `CenterSpringRuntimeState` so initial local child positions, center-space tail state, direct spring-chain children, first scene-child fallback, VRM0 root traversal expansion, and VRM0 final-joint fallback are captured separately from mutable frame state. The parity path reads the selected child's local translation to match three-vrm's `child.position`, can use exact world matrices for collider offset materialization, synchronizes world transforms after each joint, and validates both stable rotations and all-joint center-tail state against ignored three-vrm golden data.
- LookAt exposes azimuth/altitude calculation and expression-weight mapping for `lookLeft`, `lookRight`, `lookUp`, and `lookDown`.
- MToon exposes renderer hints such as render order and outline enablement, but not backend-specific shader generation.
- Adapter code provides `HumanoidPoseRig` and pose snapshots for raw/normalized pose workflows, headless first-person mesh planning, MToon material descriptors and renderer material plans, `LookAtAccess` for VRMA lookAt writeback, and `VrmRuntimeDriver` for engines that want one high-level tick over animation frames, runtime events, constraints, spring bone, first-person visibility, VRM0 orientation compatibility, and material hints. `HumanoidPoseRig` follows three-vrm's normalized humanoid hierarchy, including thumb metacarpal/proximal parenting. VRMA humanoid application uses normalized pose writeback before raw engine transforms are mutated, matching three-vrm's animation clip flow. `tick_with_spring_parity` is the preferred high-level spring path when the engine can provide an initial `SpringRestMap`.
- MToon pipeline data is exposed as pass hints for renderer-side pipeline selection; shader generation remains outside `vrm-core`.
- The concrete render-capture examples share a backend-neutral `CaptureMaterialPlan` for MToon/glTF alpha mode, culling, depth-write, blend, render-order, and transparent-order decisions. wgpu and Bevy keep only the final API-specific conversion at the edge, which keeps material policy parity testable without tying the rules to one renderer.

## Test Fixture Policy

Concrete sample data is generated in tests rather than committed as model files. This gives us repeatable VRM-shaped data without adding binary assets or license baggage to the repository.

## Local Tooling Boundaries

Local automation should keep IO at the edge even when it lives outside the library crates. `tools/ci/local-ci.rs` is allowed to read and write `.external-fixtures/`, spawn renderer/reference commands, and create PNG/HTML/Markdown artifacts, but artifact interpretation is kept in small side-effect-free helpers where practical. RGBA JSON parsing, alpha statistics, diff-heatmap pixel generation, PSNR report summary extraction, and render summary Markdown construction operate on strings, JSON values, or in-memory RGBA buffers before the runner writes files. This keeps the render-parity harness reusable for future non-Bevy engines and makes failures easier to test without launching three-vrm, wgpu, or Bevy.
