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
- Raised generated/unit/doc coverage to 83.16% line coverage after pose, spring parity, KHR emissive, and first-person auto tests.
- Added first-person `auto` topology handling in `vrm-adapter`: head descendants are hidden in first-person view and visible in third-person view.
- Added renderer-agnostic first-person headless skinned-mesh planning APIs: skin influence inspection, triangle erasure planning, and adapter operations for third-person original plus first-person clone.
- Added `VRMC_springBone_extended_collider` `inside` mapping and inside collision resolution for spring colliders.
- Added humanoid pose parity snapshot/diff helpers for real-avatar raw/normalized numeric comparisons.
- Added VRMA loader warning diagnostics for missing/draft/unknown `specVersion` and ignored non-hips humanoid translation tracks.
- Added MToon material descriptor generation and a Bevy 0.18.1 adapter skeleton crate with node/asset registries and MToon descriptor bridge.
- Raised generated/unit/doc coverage to 83.70% line coverage after parity API and Bevy skeleton tests.
- Added `CenterSpringRuntimeState`, `SpringRestMap`, `step_spring_bone_system_parity`, and `VrmRuntimeDriver::tick_with_spring_parity` so engine adapters can drive spring bones through the center-space three-vrm parity path instead of the older world-space particle path.
- Spring rest capture now records sparse spring-chain children, first scene-child fallback, center-space initial tails, center node references, and the VRM0 7cm final-joint fallback separately from mutable runtime state.
- Fixed spring parity rest capture to use selected child local translation like three-vrm's `child.position`, made zero-delta parity stepping a no-op at the adapter layer, and added `WorldTransformUpdate` so high-level parity ticks can synchronize engine world matrices before spring simulation.
- Added `vrm-io` rest scene graph output (`GltfSceneRest`) with parent/children/local/world transforms so adapter parity tests can use real glTF/VRM node data.
- Made external fixture tests recursive and added an ignored adapter test that captures `SpringRestMap` and steps center-space spring parity on real external VRM files.
- Recorded additional external-only official sample files and license notes. Even when embedded VRM metadata permits redistribution, these binary assets are kept out of the MIT/Apache repository.
- Added `tools/three-vrm-golden.mjs` to generate ignored three-vrm spring golden JSON from real VRM files, plus an ignored adapter comparison test for stable-length Seed-san spring joint rotations over multiple frames.
- Updated parity stepping to synchronize world transforms after each spring joint rotation so downstream sparse-chain joints see the same immediately-updated world matrices as three-vrm.
- Extended three-vrm spring golden output with private center-space tail state and compare it in the ignored parity test. This covers tiny-tail joint simulation state even when quaternion output is too sensitive to normalize directly.
- Extended the same ignored three-vrm golden file with raw and normalized humanoid rest/current poses, added a Seed-san pose parity test, and fixed the thumb proximal parent map to match three-vrm/spec normalized hierarchy.
- Added deterministic three-vrm posed humanoid writeback scenarios for raw and normalized pose APIs. The ignored parity test now verifies Rust raw-pose writeback and normalized-to-raw writeback against three-vrm raw absolute output on Seed-san.
- Added ignored spring golden directory parity so multiple external three-vrm spring golden files can be compared together. The local set now covers Seed-san's center-node springs and VRM1_Constraint_Twist_Sample's collider-heavy springs.
- Started VRMA full application parity by adding `LookAtAccess`, `apply_look_at_frame`, and `apply_animation_frame_with_look_at` so sampled VRMA lookAt rotations can be delivered through the renderer-agnostic adapter path alongside humanoid and expression tracks.
- Completed the first external VRMA application parity loop: `tools/three-vrm-vrma-golden.mjs` applies `test.vrma` to Seed-san through three-vrm, and an ignored Rust test compares sampled/application output for raw humanoid pose, hips translation, expression weights, and lookAt quaternion. Rust VRMA humanoid application now uses `HumanoidPoseRig` normalized-to-raw writeback to match three-vrm's normalized rig flow.
- Completed the next ordered parity push:
  1. Broadened VRMA application parity beyond a single raw-pose assertion by comparing normalized pose output and allowing directory-level VRMA golden files.
  2. Deepened VRM0 compatibility coverage for legacy first-person flag spelling and lookAt range mapping.
  3. Tightened spring numeric parity reporting by collecting per-golden maximum tail and rotation deltas, with stricter tail tolerance for normal fixtures and documented wider tolerance for collider-heavy fixtures.
- Addressed pessimistic gpt-5.4 review findings for that push: spring tail reporting now uses component-wise deltas to match the assertion semantics, VRMA normalized pose parity is reconstructed from the written raw scene, VRMA expression parity rejects unexpected Rust-only expression keys, VRM0 compatibility tests cover lowercase/unknown first-person flags plus all lookAt range directions/defaults, and testing docs now describe current coverage instead of stale next-priority text.
- Started the next implementation slice:
  1. Broaden external fixture semantic assertions for VRM0/VRMA/MToon/expression official samples without committing model binaries.
  2. Keep VRMA breadth extensible by asserting track-class coverage per external `.vrma` file.
  3. Add a minimal Bevy runtime plugin/config marker so downstream Bevy integrations have an ECS entry point before full transform/material writeback exists.
- Completed that slice by extending ignored external IO fixture assertions for MToon UV animation, expression override samples, constraints, spring bones, optional VRM0 samples, and VRMA multi-track extraction, and by adding `VrmRuntimePlugin` plus `BevyVrmRuntimeConfig` to `vrm-adapter-bevy`.
- Started the next parity slice:
  1. Add a local-only VRM0 Alicia fixture from UniVRM for ignored compatibility assertions without committing the binary asset.
  2. Port representative three-vrm node constraint solver cases for rotation, roll, and aim constraints.
  3. Refresh docs and coverage after the new parity cases pass.
- Added ignored external VRM0 semantic assertions for `AliciaSolid_vrm-0.51.vrm`, covering `VrmKind::Vrm0Compat`, compatibility metadata presence, and user-facing first-person/expression feature availability.
- Expanded VRM0 protocol compatibility to accept object-form vec3 values (`{ x, y, z }`) and legacy `-1` sentinel indices for optional texture/bone/spring center references, as required by the Alicia VRM0 sample.
- Added three-vrm-derived node constraint parity tests for rotation rest/weight cases, roll axis/rest cases, and aim axis/parent/weight cases in `vrm-runtime`.
- Raised generated/unit/doc coverage to 77.03% line coverage after the Alicia VRM0 protocol test and node constraint solver parity cases.
- Continued the Alicia VRM0 compatibility pass with semantic assertions for normalized humanoid aliases, Auto first-person mesh annotations, Bone lookAt ranges, legacy expression preset aliases, VRM0 MToon material slots/outline, and secondary animation spring/collider counts.
- Fixed VRM0 expression preset alias mapping so legacy names such as `a`, `i`, `u`, `joy`, `sorrow`, `fun`, `lookup`, and `blink_l` map to canonical VRM1-style expression keys instead of becoming custom expressions.
- Re-measured coverage after the ignored Alicia semantic assertions: workspace line coverage is 76.45%, still above the conservative 70% CI gate. `vrm-sans-io` coverage rose to 91.90% while `vrm-io` line coverage dropped because the new external-only assertions are intentionally ignored in normal coverage runs.
- Expanded renderer-facing MToon material descriptors with base color, emissive factor, cutoff, receive shadow rate, shading grade, light color attenuation, matcap, parametric rim, rim lighting, and outline lighting parameters.
- Mapped those additional MToon parameters from VRM0 legacy material float/vector properties and VRMC_materials_mtoon 1.0 where the spec exposes them. Alicia external fixture assertions now check the VRM0 legacy values on a real material.
- Re-measured coverage after the MToon descriptor expansion: workspace line coverage is 76.78%, with `vrm-sans-io` at 92.41% and `vrm-core` at 78.06%.

Open work:

- Current ordered parity push requested on 2026-04-29:
  1. Add posed humanoid writeback golden scenarios against three-vrm. Done for Seed-san raw and normalized writeback.
  2. Add collider-heavy, center-node, and additional fixture spring bone golden parity. Done for Seed-san plus VRM1_Constraint_Twist_Sample external golden files.
  3. Add full VRMA model-application parity for humanoid, hips translation, expression, and lookAt tracks. Done for Seed-san plus `test.vrma`.
- Latest ordered parity push requested on 2026-04-29:
  1. Expand VRMA parity. Done with normalized pose comparison plus directory-level VRMA golden test.
  2. Expand VRM0 compatibility parity. Done for legacy first-person flags and lookAt ranges.
  3. Improve spring numeric parity. Done with per-golden max-delta reporting and explicit tolerance classes.
- Deep VRM0 compatibility parity beyond root orientation now has an external Alicia fixture semantic check for normalized humanoid aliases, mesh annotations, lookAt, expression aliases, MToon material properties, and secondary animation. Remaining work is numeric humanoid-axis parity and the long tail of legacy material value edge cases not yet represented in the renderer-facing descriptor.
- Humanoid pose parity now has snapshot/diff helpers plus Seed-san raw/normalized rest-state and posed writeback golden coverage.
- Spring bone parity now has stable-length multi-frame rotation comparison plus all-joint center-tail state comparison on Seed-san and directory-level coverage for the collider-heavy VRM1_Constraint_Twist_Sample fixture. Remaining spring work is deeper solver investigation for the small collider-heavy numeric tolerance and more official sample breadth.
- Node constraint parity now includes direct three-vrm quaternion cases for rotation, roll, and aim solvers. Remaining work is fixture-driven manager ordering/writeback parity on complete VRM scenes.
- Renderer-specific MToon shader materialization in downstream adapters.
- Full VRMA model application parity now has one external numeric fixture comparison; remaining work is broader VRMA fixture coverage and stricter channel/path diagnostics.
- First-person `auto` has headless split planning, but downstream engines still need concrete mesh clone implementations.
- MToon pipeline/shader generation per renderer.
- Real Bevy runtime trait implementations beyond the current registry/descriptor skeleton.
