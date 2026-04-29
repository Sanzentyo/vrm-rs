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
- External fixture tests assert semantic presence for official samples and compare Seed-san spring and humanoid rest-state output against three-vrm; posed humanoid writeback coverage is still pending.
- Adapter tests use mock engines; Bevy/wgpu/ash compile examples are still pending.
- Renderer-specific MToon shader generation is intentionally outside current coverage.

## Current Coverage Snapshot

Measured locally on 2026-04-29 with:

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
```

| Scope | Region coverage | Line coverage |
| --- | ---: | ---: |
| Workspace total | 76.84% | 80.67% |
| `vrm-adapter-bevy` | 69.14% | 66.04% |
| `vrm-adapter` | 73.25% | 81.78% |
| `vrm-core` | 67.63% | 76.73% |
| `vrm-io` | 73.71% | 69.81% |
| `vrm-protocol` | 89.01% | 85.20% |
| `vrm-runtime` | 79.32% | 79.70% |
| `vrm-sans-io` | 85.82% | 88.89% |
| facade `src/lib.rs` | 96.30% | 100.00% |

The next test-effort priority is additional real-avatar numeric golden comparison against three-vrm, especially posed humanoid writeback behavior and collider-heavy spring fixtures. The current external fixture tests now cover recursive fixture discovery, semantic IO loading, adapter spring rest capture/stepping, Seed-san center-space spring golden comparison, and Seed-san raw/normalized humanoid rest-state comparison on real VRM files without committing those binaries.

## three-vrm Golden Generation

Build the local sibling `../three-vrm` workspace first, then generate spring golden output into the ignored fixture area:

```powershell
npx pnpm@10.24.0 install
npx pnpm@10.24.0 --filter @pixiv/three-vrm-springbone --filter @pixiv/three-vrm-core --filter @pixiv/three-vrm-materials-mtoon --filter @pixiv/three-vrm-materials-hdr-emissive-multiplier --filter @pixiv/three-vrm-materials-v0compat --filter @pixiv/three-vrm-node-constraint --filter @pixiv/three-vrm --filter @pixiv/three-vrm-animation build
node tools\three-vrm-golden.mjs --fixture D:\git\vrm-rs\.external-fixtures\official\Seed-san.vrm --three-vrm-root D:\git\three-vrm --frames 8 --out D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json
```

Run the ignored comparison with:

```powershell
$env:VRM_RS_THREE_VRM_GOLDEN = "D:\git\vrm-rs\.external-fixtures\golden\Seed-san.spring.json"
cargo test -p vrm-adapter spring_parity_matches_three_vrm_golden_rotations -- --ignored
cargo test -p vrm-adapter humanoid_pose_matches_three_vrm_golden_rest_state -- --ignored
```

The golden output records public local rotations, three-vrm's private center-space spring tail state, and humanoid raw/normalized rest/current poses. The spring comparison checks center tails for all joints over multiple frames, including tiny-tail joints, and compares rotations only for stable-length joints. Extremely tiny tail vectors (`<= 0.001`) are skipped for quaternion comparison because their normalized direction is numerically sensitive, but their simulation state remains covered by the center-tail assertion.

## Current External Official Samples

Spark downloaded the current local fixture set into `.external-fixtures/official/` on 2026-04-29. These files are intentionally ignored by git.

| File | Source | Local use note |
| --- | --- | --- |
| `Seed-san.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/Seed-san/vrm/Seed-san.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `VirtualCast, Inc.`; keep external because it is not MIT/Apache source code. |
| `VRM1_Constraint_Twist_Sample.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; keep external. The three-vrm mirror is byte-identical. |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_materials_mtoon_UV_Animation_Test/vrm/VRMC_materials_mtoon_UV_Animation_Test.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for MToon UV animation parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overridden/vrm/VRMC_vrm_expressions_isBinary_Overridden.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRMC_vrm_expressions_isBinary_Overrides/vrm/VRMC_vrm_expressions_isBinary_Overrides.vrm` | Embedded VRM metadata says VRM Public License 1.0, `allowRedistribution=true`, author `pixiv Inc.`; useful for expression override parity, external only. |
| `test.vrma` | `https://raw.githubusercontent.com/pixiv/three-vrm/9d125586f6d7da094b0ac5f204cebf19586f2397/packages/three-vrm-animation/examples/models/test.vrma` | Local testing only until upstream redistribution status is confirmed; no embedded asset license/provenance found. |

The URLs are commit-pinned to avoid branch drift.
