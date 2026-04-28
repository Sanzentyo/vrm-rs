# Progress

## 2026-04-29

- Created workspace skeleton.
- Added `vrm-protocol` schema coverage for VRM0, VRMC_vrm, VRMC_springBone, VRMC_node_constraint, VRMC_materials_mtoon, and VRMC_vrm_animation.
- Added `vrm-core` domain types, state markers, newtype references, feature ADT, humanoid/expression/lookAt/spring/constraint/material/animation models.
- Added `vrm-sans-io` conversion and validation.
- Added `vrm-io` glTF import and extension extraction.
- Added `vrm-runtime` expression, constraint ordering, spring ordering, lookAt math, and track sampling.
- Added `vrm-adapter` engine integration traits and mock adapter test.
- Added generated in-test glTF/VRM sample data that is not stored as repository model assets.
- Added IO tests that load generated VRMC_vrm, VRMC_springBone, VRMC_node_constraint, and VRMC_materials_mtoon data.
- Expanded validation to expression node/material references and spring collider/collider-group references.
- Switched runtime expression keys to VRM canonical names such as `blink` and `lookLeft`.

Open work:

- Deep VRM0 compatibility parity.
- Full spring bone Verlet simulation and collision response.
- Full VRMA glTF channel extraction.
- MToon pipeline/shader generation per renderer.
- Real Bevy adapter crate once a Bevy version is selected.
