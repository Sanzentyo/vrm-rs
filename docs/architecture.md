# Architecture

## Crates

- `vrm-protocol`: serde wire types for VRM 0.0, VRMC_vrm 1.0, VRMC_springBone, VRMC_node_constraint, VRMC_materials_mtoon, KHR emissive strength, and VRMC_vrm_animation.
- `vrm-core`: pure domain types. This crate defines `VrmAsset<State>`, `VrmModel<State>`, `NodeRef`, `MaterialRef`, `TextureRef`, humanoid, expressions, lookAt, spring bone, constraints, material parameters, and animation tracks.
- `vrm-sans-io`: side-effect-free conversion from protocol data into validated core data.
- `vrm-io`: glTF/GLB IO through the `gltf` crate, extension extraction, buffer/image collection, and model construction.
- `vrm-runtime`: renderer-independent update orchestration and algorithms.
- `vrm-adapter`: traits for scene graph, transforms, morph targets, materials, textures, and animation sinks.
- `vrm-rs`: facade crate.

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

Current channel mapping:

- Humanoid `rotation` channels become per-bone `RotationTrack`s after rest-pose aware remapping.
- `hips` `translation` channels become `hips_translation` after mapping through the hips parent rest matrix.
- Expression `translation` channels use the X component as scalar expression weight.
- lookAt `rotation` channels become `look_at_track`.

`VrmAnimation::rest_hips_position` captures the hips node rest world position. `vrm-runtime` samples clips into `VrmAnimationFrame`, and `vrm-adapter` applies those frames through `TransformAccess`, `MorphTargetAccess`, and `MaterialAccess` without depending on Bevy, wgpu, ash, or glTF node objects.

## Runtime Solvers

Runtime math remains renderer-agnostic. Engine adapters are expected to provide current transforms and apply returned rotations/positions.

- Node constraints expose pure solvers for rotation, roll, and aim constraints.
- Spring bone exposes world-space particle state, typed center-space parity particle state, rest state, Verlet-style step helpers, center-space parity stepping, and sphere/capsule/plane collision correction.
- LookAt exposes azimuth/altitude calculation and expression-weight mapping for `lookLeft`, `lookRight`, `lookUp`, and `lookDown`.
- MToon exposes renderer hints such as render order and outline enablement, but not backend-specific shader generation.
- Adapter code provides `HumanoidPoseRig` for raw/normalized pose read/write workflows and `VrmRuntimeDriver` for engines that want one high-level tick over animation frames, runtime events, constraints, spring bone, first-person visibility, VRM0 orientation compatibility, and material hints.
- MToon pipeline data is exposed as pass hints for renderer-side pipeline selection; shader generation remains outside `vrm-core`.

## Test Fixture Policy

Concrete sample data is generated in tests rather than committed as model files. This gives us repeatable VRM-shaped data without adding binary assets or license baggage to the repository.
