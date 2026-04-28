# Intent

User request, recorded on 2026-04-29 Asia/Tokyo:

- Port the ideas in `../three-vrm` into a Rust library for working with VRM.
- Do not mechanically reproduce the TypeScript implementation; design Rust-first APIs.
- Cover IO, parsing, protocol/schema data, sans-IO layers, runtime handling, and external framework usage.
- Provide a `core`/`protocol`-style sans-IO layer and concrete layers above it.
- Use ADTs, newtypes, and Type State Pattern actively.
- Keep progress and decisions in `docs/`.
- Use subagents and DeepWiki during planning and implementation.
- Target use from Bevy and non-Bevy engines, including custom wgpu/ash projects.

Chosen implementation defaults:

- Workspace split.
- Initial milestone: parse + renderer-agnostic runtime core.
- Integration strategy: trait adapters first, optional Bevy skeleton only.
