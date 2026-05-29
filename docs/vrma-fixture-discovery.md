# VRMA Fixture Discovery

This document records source and license checks for `.vrma` clips used by parity
tests. Binary clips stay under `.external-fixtures/` unless redistribution is
explicitly reviewed for this repository.

## Current Finding

As of 2026-05-29, one stable public upstream `.vrma` sample and one
experimental branch-only upstream `.vrma` sample were found:

| Repository | Commit | Path | Status |
| --- | --- | --- | --- |
| `pixiv/three-vrm` | `9d125586f6d7da094b0ac5f204cebf19586f2397` | `packages/three-vrm-animation/examples/models/test.vrma` | Used as an external-only parity clip. Keep out of git until asset provenance is reviewed separately from the repository MIT license. |
| `pixiv/three-vrm` | `75ab65c9d4e488521d41bff7f5cfd1976a0b16e8` | `packages/vrm-viewer/examples/models/idle_loop.vrma` | Experimental branch-only sample from `feat/vrm-viewer-experimental-vrma`. Used as external-only parity coverage because it revealed hips-translation scale behavior not covered by `test.vrma`. Do not vendor without explicit review. |

Checked repositories:

- `pixiv/three-vrm`
  - Pinned test commit: `9d125586f6d7da094b0ac5f204cebf19586f2397`
  - Latest checked `release` commit: `54e050311b9a27881da21ab842e15380bb512ad8`
  - Experimental viewer commit checked by Spark: `75ab65c9d4e488521d41bff7f5cfd1976a0b16e8`
  - Result: stable `packages/three-vrm-animation/examples/models/test.vrma`, plus branch-only `packages/vrm-viewer/examples/models/idle_loop.vrma`.
- `vrm-c/vrm-specification`
  - Pinned sample commit: `3942748efbc803b258e288e0f6c993c6bb96cebf`
  - Result: no `.vrma` files in the tree.
- `vrm-c/UniVRM`
  - Latest checked `master` commit: `2b5ebcf3d793f7a853e31412cd1c32b4f79a6962`
  - Result: VRMA import/export tooling and Unity samples exist, but no committed `.vrma` sample files were found.
- `vrm-c/bvh2vrma`
  - Latest checked `main` commit: `da148d9a377739cef91c1a1e57d56d381a88aadc`
  - Result: converter tooling exists, but no committed `.vrma` sample files were found.

## Recheck Commands

Use GitHub's tree API so the search is deterministic and does not depend on a
local clone:

```powershell
gh api repos/pixiv/three-vrm/git/trees/9d125586f6d7da094b0ac5f204cebf19586f2397?recursive=1 --jq '.tree[].path' | Select-String -Pattern '\.vrma$'
gh api repos/pixiv/three-vrm/git/trees/54e050311b9a27881da21ab842e15380bb512ad8?recursive=1 --jq '.tree[].path' | Select-String -Pattern '\.vrma$'
gh api repos/pixiv/three-vrm/contents/packages/vrm-viewer/examples/models/idle_loop.vrma?ref=75ab65c9d4e488521d41bff7f5cfd1976a0b16e8 --jq '{name,path,size,download_url,sha}'
gh api repos/vrm-c/vrm-specification/git/trees/3942748efbc803b258e288e0f6c993c6bb96cebf?recursive=1 --jq '.tree[].path' | Select-String -Pattern '\.vrma$'
gh api repos/vrm-c/UniVRM/git/trees/2b5ebcf3d793f7a853e31412cd1c32b4f79a6962?recursive=1 --jq '.tree[].path' | Select-String -Pattern '\.vrma$'
gh api repos/vrm-c/bvh2vrma/git/trees/da148d9a377739cef91c1a1e57d56d381a88aadc?recursive=1 --jq '.tree[].path' | Select-String -Pattern '\.vrma$'
```

## Next Parity Options

- Keep `test.vrma` as the official upstream external clip for three-vrm
  application parity.
- Keep `idle_loop.vrma` as branch-only external parity coverage while it remains
  outside release/dev. It is useful because its hips translation requires
  three-vrm's rest-hips-height scaling behavior.
- Add generated, source-like VRMA fixtures in tests for broader track coverage
  when no additional official clip exists. These should remain JSON generated in
  tests or generated into `.external-fixtures/`, not committed as binary clips.
- Consider a reproducible bvh2vrma conversion pipeline only if the source BVH
  clip has clear redistribution permission and the generated VRMA remains
  external-only or is explicitly approved for vendoring.
