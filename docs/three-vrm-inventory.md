# three-vrm Inventory

`../three-vrm` is a TypeScript monorepo with these relevant seams:

- `packages/types-vrm-0.0`: VRM 0.0 schema declarations.
- `packages/types-vrmc-vrm-1.0`: VRMC_vrm 1.0 schema declarations.
- `packages/types-vrmc-springbone-1.0`: spring bone extension declarations.
- `packages/types-vrmc-node-constraint-1.0`: node constraint extension declarations.
- `packages/types-vrmc-materials-mtoon-1.0`: MToon material extension declarations.
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

- VRM0 compatibility has axis/name quirks and needs fixture-heavy testing.
- MToon shader/pipeline behavior is renderer-specific; the first Rust API exposes parameters and hints only.
- Spring bone physics needs deeper parity work beyond first deterministic update ordering.
- Node constraints should error on circular dependencies.
