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
- Added `load_vrm_from_path` and ignored external fixture directory tests using `VRM_RS_FIXTURE_DIR`.
- Ignored `.external-fixtures/` for local official sample downloads.
- Downloaded official/local-testing samples into `.external-fixtures/official/` via Spark and verified `cargo test -p vrm-io loads_external_fixture_directory -- --ignored`.
- Added VRMA glTF animation channel extraction for humanoid rotation, hips translation, expression scalar tracks, and lookAt rotation tracks.
- Added `VrmDocument::animations` to preserve multiple glTF animation clips while keeping `animation` as the first/legacy convenience value.
- Added VRM0 `firstPerson`/lookAt compatibility mapping and `secondaryAnimation` to spring bone normalization.
- Added pure spring particle Verlet-style stepping and sphere/capsule/plane collision correction helpers in `vrm-runtime`.
- Added renderer-agnostic node constraint solvers for rotation, roll, and aim constraints.
- Added MToon render queue hints, `render_order()`, and `outline_enabled()` helpers.
- Added VRMA rest-pose aware rotation remapping, rest hips position capture, and hips translation mapping through the hips parent rest matrix.
- Added `sample_vrm_animation` frame application helpers in `vrm-adapter` so engine adapters can apply sampled humanoid rotations, hips translation, and expression binds through traits.
- Added renderer-agnostic LookAt expression weight calculation for `lookLeft`, `lookRight`, `lookUp`, and `lookDown`.
- Added first-person visibility adapter helpers that map VRM mesh annotations to engine-provided node visibility.
- Added spring bone collider group resolution, world/center-space collider conversion, and adapter helpers for collecting simulation-space colliders from engine world transforms.
- Added node constraint adapter application helpers that read current/rest transforms through traits and write solved local rotations back to the target engine.
- Added spring tail-to-local-rotation solving and adapter writeback helper for applying solved spring tails to engine joint transforms.
- Added `ConstraintRestMap` capture helper so adapters can snapshot node constraint rest rotations from initial local transforms.
- Added high-level spring bone adapter stepping that gathers colliders, advances particles, and writes joint rotations through engine transform traits.
- Expanded VRM0 compatibility mapping for blend shape material value binds and legacy thumb intermediate bone aliases.
- Expanded VRM0 blend shape material value mapping so texture transform properties such as `_MainTex_ST` become `TextureTransform` binds instead of generic material color binds.
- Added renderer-agnostic MToon texture slots in core and mapped VRM0 texture properties plus VRMC_materials_mtoon texture infos into them.
- Finished the first explicit VRM0 orientation compensation layer as `Vrm0Compatibility::orientation_correction`, with adapter helper support for applying it to an engine root node.
- Added renderer-agnostic MToon pipeline hints for alpha mode, culling, depth write/test, blend, render order, and outline pass hints.
- Added `VrmRuntimeDriver` in `vrm-adapter` to combine VRM0 orientation, animation frame application, runtime expression events, constraints, spring stepping, MToon pipeline hints, and first-person visibility in one tick.
- Added MToon pipeline pass helper and adapter trait for renderer-side pipeline selection without shader generation.
- Added CI workflow for fmt/test/clippy and non-threshold `cargo-llvm-cov` summary coverage.
- Expanded external fixture tests from load-only checks to semantic assertions for VRM metadata/humanoid/material/spring/constraints and VRMA tracks.
- Expanded protocol roundtrip tests for VRM0, spring bone, node constraint, MToon, VRMA, unknown extensions, and invalid extension errors.
- Hardened `VrmRuntimeDriver` so VRM0 root orientation compensation is applied once per driver instead of compounding every tick.
- Changed VRMA/sample humanoid hips translation adapter writeback to set absolute local translation for the sampled frame rather than accumulating deltas.
- Added per-extension `specVersion` validation for extracted `VRMC_springBone`, `VRMC_node_constraint`, and `VRMC_materials_mtoon`, accepting three-vrm-compatible `1.0` and `1.0-beta`.
- Added archived `VRMC_materials_hdr_emissiveMultiplier` protocol/IO/sans-IO/core/adapter support, mapping it to renderer-facing emissive intensity.
- Fixed VRM1 material extension mapping to preserve glTF material indices in `VrmDocument::materials`.
- Raised generated/unit/doc coverage to 81.02% line coverage and added a conservative CI `cargo-llvm-cov --fail-under-lines 70` gate.
- Added type-state style humanoid pose types for raw/normalized and absolute/rest-relative poses.
- Added `HumanoidPoseRig` in `vrm-adapter` for raw pose get/set/reset, normalized pose get/set/reset, and normalized-to-raw writeback through engine transform traits.
- Added spring bone rest-state and center-space parity stepping helpers mirroring three-vrm's tail integration shape, including VRM0 7cm final-joint fallback, center-space particle state typing, and initial-local-rotation premultiplication.
- Added `KHR_materials_emissive_strength` protocol/IO/sans-IO/core/adapter support and made it take precedence over archived `VRMC_materials_hdr_emissiveMultiplier`.
- Raised generated/unit/doc coverage to 82.91% line coverage after pose, spring parity, and KHR emissive tests.

Open work:

- Deep VRM0 compatibility parity beyond root orientation, especially fixture-driven edge cases.
- Humanoid pose parity still needs numeric fixture comparison against three-vrm on real avatars.
- Spring bone parity still needs multi-joint sparse-chain numeric comparison against three-vrm.
- Renderer-specific MToon shader materialization in downstream adapters.
- Full VRMA model application parity with stricter channel/path diagnostics and numeric fixture comparisons.
- First-person `auto` handling that uses mesh/head topology instead of the current conservative visible-both behavior.
- MToon pipeline/shader generation per renderer.
- Real Bevy adapter crate once a Bevy version is selected.
