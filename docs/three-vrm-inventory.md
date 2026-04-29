# three-vrm Inventory

`../three-vrm` is a TypeScript monorepo with these relevant seams:

- `packages/types-vrm-0.0`: VRM 0.0 schema declarations.
- `packages/types-vrmc-vrm-1.0`: VRMC_vrm 1.0 schema declarations.
- `packages/types-vrmc-springbone-1.0`: spring bone extension declarations.
- `packages/types-vrmc-node-constraint-1.0`: node constraint extension declarations.
- `packages/types-vrmc-materials-mtoon-1.0`: MToon material extension declarations.
- `packages/types-vrmc-materials-hdr-emissive-multiplier-1.0`: archived HDR emissive multiplier material extension declarations.
- `KHR_materials_emissive_strength`: Khronos material emissive strength extension used instead of the archived VRMC multiplier when present.
- `packages/types-vrmc-vrm-animation-1.0`: VRMA extension declarations.
- `packages/three-vrm-core`: humanoid, expressions, first-person, lookAt, meta.
- `packages/three-vrm-springbone`: spring bone manager, colliders, dependency ordering.
- `packages/three-vrm-node-constraint`: roll/aim/rotation constraints and dependency update ordering.
- `packages/three-vrm-materials-mtoon`: MToon material parameter mapping and renderer-specific material setup.
- `packages/three-vrm-animation`: VRMA mapping to animation tracks.
- `packages/three-vrm`: aggregate loader and runtime facade.

Design translation:

- Type packages become `vrm-protocol`.
- Loader plugin lifecycle becomes explicit IO + sans-IO conversion rather than Three.js plugin hooks.
- Runtime managers become renderer-agnostic event/order producers.
- Renderer mutations are pushed into `vrm-adapter` traits.

Known risks:

- VRM0 compatibility has axis/name quirks and needs fixture-heavy testing; root orientation and humanoid pose API shape are covered, but real-avatar numeric parity is still pending.
- MToon shader/pipeline behavior is renderer-specific; the first Rust API exposes parameters and hints only.
- Spring bone physics now has center-space parity helpers, but still needs real three-vrm numeric fixture comparison.
- Node constraints should error on circular dependencies.

Compatibility checkpoints added:

- `VRMC_materials_hdr_emissiveMultiplier` now roundtrips as protocol data, maps to `HdrEmissiveMultiplier`, survives glTF material index placement, and can be written to engine material state through `MaterialAccess::set_emissive_intensity`.
- `KHR_materials_emissive_strength` now roundtrips, maps to `EmissiveStrength`, and takes precedence over archived VRMC HDR multiplier.
- `HumanoidPoseRig` covers raw/normalized absolute and rest-relative pose workflows using engine transform traits.
- Spring rest-state parity helpers cover typed center-space tails, initial-local-rotation premultiplication, and VRM0 7cm childless-joint fallback.
- Per-node/per-material VRMC extension `specVersion` validation now accepts `1.0`/`1.0-beta` and rejects unsupported versions for spring bone, node constraint, and MToon.
- `VrmRuntimeDriver` treats VRM0 root orientation compensation as a one-shot driver state, matching loader-style behavior instead of compounding every runtime tick.
- VRMA hips translation writeback is absolute per sampled frame, matching animation-track semantics instead of accumulating deltas.
