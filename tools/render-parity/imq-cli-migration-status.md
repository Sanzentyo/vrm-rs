# imq CLI Migration Status

## Status

**Blocked.** The public `imq` CLI is not yet the render-parity acceptance comparator because the repository documentation records that the required VRM-specific domains and threshold gates are not available in the checked public CLI revision.

## Current Canonical Path

- Numeric input: direct renderer `.imqraw` artifacts.
- Numeric comparator: `tools/render-parity/compare-imqraw.rs`.
- Report contract: `vrm-rs.render-parity.imqraw-comparison` version `1`.
- Companion `.rgba.json`: visual review, debugging, PNG/diff generation, and byte-consistency verification through `verify-imqraw-rgba.rs` only.
- JavaScript `.rgba.json` comparator: retired; it is not a fallback or alternate mode.

## Cutover Rule

The public `imq` CLI may replace the repository-local Rust comparator only when every required capability in `imq-cli-migration-requirements.json` is implemented and verified. The cutover must happen in one change:

1. Change the single numeric-comparator invocation in `tools/ci/local-ci.rs`.
2. Update the report validator to the public CLI's final structured schema in the same change.
3. Delete `tools/render-parity/compare-imqraw.rs` in that change.
4. Do not add `imq-cli`, `legacy-js`, fallback, wrapper, alias, or dual-report modes.
5. Re-run the Ash-gated sample lane, acceptance repeat, goal-readiness audit, and multi-environment acceptance aggregation.

## Open Requirements

- Direct `.imqraw` RGBA8 input with record-index selection and strict format/dimension validation.
- All VRM metric domains and one-, two-, and three-pixel masks listed in the JSON requirements.
- Alpha bucket/mismatch diagnostics and changed-pixel/high-delta diagnostics.
- Selected metric, PSNR floor, selected max-channel-delta, and alpha-delta gates with a failing process status.
- Machine-readable JSON carrying the source paths, selected metric, thresholds, diagnostics, and final pass state.
- Deterministic top-left RGBA8 semantics without PNG or `.rgba.json` numeric conversion.

## Evidence Still Required

No migration or post-cleanup acceptance command was executed as part of this static task. The required commands are listed in `review/unverified-checks.md`.
