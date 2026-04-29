# Testing

## Generated Sample Data

The repository should not vendor third-party `.vrm`, `.vrma`, `.glb`, or texture assets for early tests. Instead, tests generate concrete minimal glTF JSON values in memory and feed them to the same public IO entrypoints used by real files.

Current generated coverage:

- Root `VRMC_vrm` 1.0 extension with meta, required humanoid bones, first-person annotations, lookAt, and expression binds.
- Root `VRMC_springBone` extension with collider, collider group, and spring joint data.
- Per-node `VRMC_node_constraint` extension.
- Per-material `VRMC_materials_mtoon` extension.
- Invalid node reference case through the same generated sample.

This keeps licensing simple while still exercising `gltf::import_slice`, extension extraction, sans-IO mapping, validation, and resolved model construction.

Later fixture strategy:

- Keep generated samples for unit and integration tests.
- Optional ignored tests read local user-provided assets from `VRM_RS_FIXTURE_DIR`, defaulting to `.external-fixtures/official`.
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

HTML report:

```powershell
cargo llvm-cov --workspace --all-features --html
```

Current known coverage gaps:

- Protocol roundtrip tests cover representative extension shapes, but not every optional schema field.
- External fixture tests assert semantic presence for official samples but do not compare numeric animation/spring outputs against three-vrm.
- Adapter tests use mock engines; Bevy/wgpu/ash compile examples are still pending.
- Renderer-specific MToon shader generation is intentionally outside current coverage.

## Current Coverage Snapshot

Measured locally on 2026-04-29 with:

```powershell
cargo llvm-cov --workspace --all-features --summary-only
```

| Scope | Region coverage | Line coverage |
| --- | ---: | ---: |
| Workspace total | 68.56% | 73.50% |
| `vrm-adapter` | 85.39% | 90.51% |
| `vrm-core` | 60.05% | 75.80% |
| `vrm-io` | 31.93% | 29.71% |
| `vrm-protocol` | 85.09% | 79.77% |
| `vrm-runtime` | 66.34% | 69.54% |
| `vrm-sans-io` | 82.73% | 85.64% |

The next test-effort priority is `vrm-io`, followed by deeper runtime numeric parity tests.

## Current External Official Samples

Spark downloaded the current local fixture set into `.external-fixtures/official/` on 2026-04-29. These files are intentionally ignored by git.

| File | Source | Local use note |
| --- | --- | --- |
| `Seed-san.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/Seed-san/vrm/Seed-san.vrm` | VRM Public License 1.0 sample, model by VirtualCast, Inc. |
| `VRM1_Constraint_Twist_Sample.vrm` | `https://raw.githubusercontent.com/vrm-c/vrm-specification/3942748efbc803b258e288e0f6c993c6bb96cebf/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm` | VRM Public License 1.0 sample, copyright note from upstream sample README. |
| `test.vrma` | `https://raw.githubusercontent.com/pixiv/three-vrm/9d125586f6d7da094b0ac5f204cebf19586f2397/packages/three-vrm-animation/examples/models/test.vrma` | Local testing only until upstream redistribution status is confirmed. |

The URLs are commit-pinned to avoid branch drift.
