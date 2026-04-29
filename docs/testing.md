# Testing

## Generated Sample Data

The repository should not vendor third-party `.vrm`, `.vrma`, `.glb`, or texture assets for early tests. Instead, tests generate concrete minimal glTF JSON values in memory and feed them to the same public IO entrypoints used by real files.

Current generated coverage:

- Root `VRMC_vrm` 1.0 extension with meta, required humanoid bones, first-person annotations, lookAt, and expression binds.
- Root `VRMC_springBone` extension with collider, collider group, and spring joint data.
- Per-node `VRMC_node_constraint` extension.
- Per-material `VRMC_materials_mtoon` extension.
- Per-material archived `VRMC_materials_hdr_emissiveMultiplier` extension.
- Per-material `KHR_materials_emissive_strength` extension, including invalid shape handling, present-but-empty defaulting, and precedence over archived HDR multiplier.
- Invalid node reference, invalid extension shape, supported `1.0-beta`, and unsupported per-extension `specVersion` cases through the same generated sample.
- First-person headless mesh triangle erasure, humanoid pose snapshot diffing, spring extended collider `inside`, VRMA warning policy, MToon descriptor generation, and Bevy adapter skeleton compile tests.

This keeps licensing simple while still exercising `gltf::import_slice`, extension extraction, sans-IO mapping, validation, and resolved model construction.

Later fixture strategy:

- Keep generated samples for unit and integration tests.
- Optional ignored tests read local user-provided assets recursively from `VRM_RS_FIXTURE_DIR`, defaulting to `.external-fixtures/official`.
- Do not commit proprietary or third-party avatar assets unless their license explicitly allows redistribution.

Run external fixture tests with:

```powershell
$env:VRM_RS_FIXTURE_DIR = ".external-fixtures/official"
cargo test -p vrm-io -- --ignored
```

`.external-fixtures/` is ignored by git so official samples can be downloaded for local validation without becoming repository source assets.

## Coverage

Line and branch coverage are measured with `cargo-llvm-cov`. The tool is not required for normal development, but release and parity work should use it when available.

Install:

```powershell
cargo install cargo-llvm-cov
```

Summary:

```powershell
cargo llvm-cov --workspace --all-features --summary-only
```

CI currently runs the same workspace coverage pass with a conservative line threshold:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

HTML report:

```powershell
cargo llvm-cov --workspace --all-features --html
```

Current known coverage gaps:

- Protocol roundtrip tests cover representative extension shapes, but not every optional schema field.
- External fixture tests assert semantic presence for official samples and compare Seed-san humanoid rest-state, posed humanoid writeback, spring center-space output, collider-heavy spring output, and VRMA application output against three-vrm.
- Runtime unit tests include representative three-vrm quaternion parity cases for node constraint rotation, roll, and aim solvers.
- Adapter tests use mock engines; Bevy/wgpu/ash compile examples are still pending.
- Renderer-specific MToon shader generation is intentionally outside current coverage.

## Current Coverage Snapshot

Measured locally on 2026-04-29 with:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

| Scope | Region coverage | Line coverage |
| --- | ---: | ---: |
| Workspace total | 72.14% | 76.45% |
| `vrm-adapter-bevy` | 72.82% | 66.67% |
| `vrm-adapter` | 62.27% | 72.46% |
| `vrm-core` | 70.36% | 76.73% |
| `vrm-io` | 65.09% | 61.16% |
| `vrm-protocol` | 86.45% | 85.40% |
| `vrm-runtime` | 82.02% | 82.69% |
| `vrm-sans-io` | 89.26% | 91.90% |
| facade `src/lib.rs` | 96.30% | 100.00% |

The current external fixture tests cover recursive fixture discovery, semantic IO loading including the Alicia VRM0 compatibility sample, adapter spring rest capture/stepping, Seed-san center-space spring golden comparison, collider-heavy spring directory comparison, Seed-san raw/normalized humanoid rest-state and posed writeback comparison, and Seed-san plus `test.vrma` application comparison on real VRM/VRMA files without committing those binaries. The Alicia VRM0 fixture asserts normalized humanoid aliases, Auto first-person annotations, Bone lookAt ranges, legacy expression preset aliases, MToon material slots/outline, and secondary animation spring/collider counts. The next test-effort priority is broader fixture breadth and stricter diagnostics, especially additional VRMA clips, numeric VRM0 humanoid-axis parity, deeper legacy material value edge cases, and renderer adapter examples.

## Ordered Parity Milestones

The current ordered work queue is:

1. Posed humanoid writeback golden: generate deterministic `setRawPose` / `setNormalizedPose` three-vrm snapshots and compare Rust writeback against the resulting raw node transforms. Done for Seed-san.
2. Spring bone fixture expansion: add collider-heavy, center-node, and non-Seed-san official fixture golden comparisons while keeping external binaries out of git. Done for Seed-san plus VRM1_Constraint_Twist_Sample.
3. VRMA application parity: apply sampled VRMA frames to a model and compare humanoid rotations, hips translation, expression weights, and lookAt tracks against three-vrm behavior. Done for Seed-san plus `test.vrma`.

Latest completed ordered work queue:

1. VRMA parity breadth: compare normalized pose reconstructed from raw scene writeback as well as raw application output, assert that Rust does not emit unexpected expression keys, and support directory-level VRMA golden files for future fixture expansion.
2. VRM0 compatibility depth: add generated compatibility tests for legacy first-person flag spelling, lowercase fallback spellings, unknown flags, all lookAt range directions, and default range values.
3. Spring numeric precision: report and assert per-golden maximum tail/rotation component deltas so simple fixtures can remain tight while collider-heavy fixtures carry an explicit wider tolerance.

Latest completed implementation slice:

1. External fixture semantic breadth: assert known official fixture features for MToon UV animation, expression override samples, constraints, spring bones, and VRMA track classes.
2. VRMA fixture breadth: keep ignored directory tests ready for additional `.vrma` files by checking humanoid rotation, hips translation, expression, and lookAt track categories when present.
3. Bevy adapter skeleton: provide a minimal plugin/config marker as the first ECS entry point before concrete transform, morph, material, and mesh writeback systems are implemented.

Current active parity slice:

1. VRM0 external compatibility fixture: load UniVRM's Alicia VRM0 sample from `.external-fixtures/official/UniVRM/` and assert compatibility-level semantics without committing the binary asset.
2. Node constraint solver parity: port representative three-vrm rotation, roll, and aim quaternion cases into `vrm-runtime` unit tests.
3. Coverage refresh: rerun fmt/test/clippy/llvm-cov after the new assertions and update the snapshot if the totals change.

Latest VRM0 Alicia expansion:

1. Legacy expression aliases map into canonical expression keys (`aa`, `ih`, `ou`, `ee`, `oh`, `happy`, `sad`, `relaxed`, `lookUp`, `blinkLeft`, etc.).
2. Ignored IO fixture assertions now cover normalized humanoid aliases, first-person mesh annotations, Bone lookAt, VRM0 MToon material mapping, and VRM0 secondary animation conversion.

Each milestone should update this document before code changes, add ignored external-fixture commands when real assets are needed, keep generated golden JSON under `.external-fixtures/`, and run the normal fmt/test/clippy/coverage gate before commit.

## three-vrm Golden Generation

Build the local sibling `../three-vrm` workspace first, then generate spring golden output into the ignored fixture area:

```powershell
npx pnpm@10.24.0 install
npx pnpm@10.24.0 --filter @pixiv/three-vrm-springbone --filter @pixiv/three-vrm-core --filter @pixiv/three-vrm-materials-mtoon --filter @pixiv/three-vrm-materials-hdr-emissive-multiplier --filter @pixiv/three-vrm-materials-v0compat --filter @pixiv/three-vrm-node-constraint --filter @pixiv/three-vrm --filter @pixiv/three-vrm-animation build
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --three-vrm-root D:\git\three-vrm --frames 8 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\VRM1_Constraint_Twist_Sample.vrm --three-vrm-root D:\git\three-vrm --frames 8 --out D:\git\vrm-rs\.external-fixtures\golden\VRM1_Constraint_Twist_Sample.spring.json
node tools\three-vrm-vrma-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --vrma D:\git\vrm-rs\.external-fixtures\official\test.vrma --three-vrm-root D:\git\three-vrm --times 0,0.5,1 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.test-vrma.json
```

Run the ignored comparison with:

```powershell
$env:VRM_RS_THREE_VRM_GOLDEN = "D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json"
cargo test -p vrm-adapter spring_parity_matches_three_vrm_golden_rotations -- --ignored
cargo test -p vrm-adapter humanoid_pose_matches_three_vrm_golden_rest_state -- --ignored
cargo test -p vrm-adapter humanoid_pose_writeback_matches_three_vrm_golden -- --ignored
$env:VRM_RS_THREE_VRM_GOLDEN_DIR = "D:\git\vrm-rs\.external-fixtures\golden"
cargo test -p vrm-adapter spring_parity_matches_three_vrm_golden_directory -- --ignored
$env:VRM_RS_THREE_VRM_VRMA_GOLDEN = "D:\git\vrm-rs\.external-fixtures\golden\Seed-san.test-vrma.json"
cargo test -p vrm-adapter vrma_application_matches_three_vrm_golden -- --ignored
$env:VRM_RS_THREE_VRM_VRMA_GOLDEN_DIR = "D:\git\vrm-rs\.external-fixtures\golden"
cargo test -p vrm-adapter vrma_application_matches_three_vrm_golden_directory -- --ignored
```

The golden output records public local rotations, three-vrm's private center-space spring tail state, humanoid raw/normalized rest/current poses, deterministic posed humanoid writeback scenarios, and VRMA application samples. The spring comparison checks center tails for all joints over multiple frames, including tiny-tail joints, and compares rotations only for stable-length joints. Extremely tiny tail vectors (`<= 0.001`) are skipped for quaternion comparison because their normalized direction is numerically sensitive, but their simulation state remains covered by the center-tail assertion. Spring tests now collect maximum tail and rotation component deltas per golden file; normal fixtures use `0.001` tail and `0.0015` rotation tolerance, while collider-heavy constraint fixtures use `0.003` tail and `0.0015` rotation tolerance because collider resolution accumulates small three.js/Rust float-path differences over chained joints. VRMA application parity compares raw humanoid pose after normalized-to-raw writeback, normalized pose reconstructed from the written raw scene, expression weights without allowing unexpected Rust-only keys, and lookAt quaternion at deterministic sample times.

## Current External Official Samples

Spark downloaded the current local fixture set into `.external-fixtures/official/` on 2026-04-29. These files are intentionally ignored by git.

| File | Source | Local use note |
| --- | --- | --- |
| `Seed-san.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/Seed-san/vrm/Seed-san.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `VirtualCast, Inc.`; keep external because it is not MIT/Apache source code. |
| `VRM1_Constraint_Twist_Sample.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; keep external. The three-vrm mirror is byte-identical. |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_materials_mtoon_UV_Animation_Test/vrm/VRMC_materials_mtoon_UV_Animation_Test.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for MToon UV animation parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overridden/vrm/VRMC_vrm_expressions_isBinary_Overridden.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overrides/vrm/VRMC_vrm_expressions_isBinary_Overrides.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `UniVRM/AliciaSolid_vrm-0.51.vrm` | `https://raw.githubusercontent.com/vrm-c/UniVRM/cc52748645889e1521f5a4cef2103b8b028100bf/Tests/Models/Alicia_vrm-0.51/AliciaSolid_vrm-0.51.vrm` | VRM0 compatibility fixture for ignored semantic tests. Keep external until redistribution/license status is reviewed for this repository's MIT/Apache source distribution. |
| `test.vrma` | `https://raw.githubusercontent.com/pixiv/three-vrm/9d125586f6d7da094b0ac5f204cebf19586f2397/packages/three-vrm-animation/examples/models/test.vrma` | Local testing only until upstream redistribution status is confirmed; no embedded asset license/provenance found. |

The URLs are commit-pinned to avoid branch drift.
