# Architecture

## Crates

- `vrm-protocol`: serde wire types for VRM 0.0, VRMC_vrm 1.0, VRMC_springBone, VRMC_node_constraint, VRMC_materials_mtoon, and VRMC_vrm_animation.
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

## Test Fixture Policy

Concrete sample data is generated in tests rather than committed as model files. This gives us repeatable VRM-shaped data without adding binary assets or license baggage to the repository.
